use assert_cmd::Command;
use parquet::data_type::ByteArray;
use parquet::file::properties::WriterProperties;
use parquet::file::writer::SerializedFileWriter;
use parquet::schema::parser::parse_message_type;
use predicates::prelude::*;
use serde_json::Value;
use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;

fn datakeel() -> Command {
    Command::cargo_bin("datakeel").expect("datakeel binary built")
}

fn example(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/seminary-etl")
        .join(rel)
}

#[test]
fn infer_odcs_and_toml_from_jsonl_and_check_passes() {
    let dir = tempdir().unwrap();
    let odcs_path = dir.path().join("students.datacontract.yaml");
    let toml_path = dir.path().join("students.toml");

    datakeel()
        .args([
            "infer",
            example("populi_students.jsonl").to_str().unwrap(),
            "--infer-enums",
            "10",
            "--contract-format",
            "odcs",
            "-o",
            odcs_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let yaml = fs::read_to_string(&odcs_path).unwrap();
    assert!(yaml.contains("apiVersion: v3.2.0"));
    assert!(yaml.contains("kind: DataContract"));
    assert!(yaml.contains("student_id"));

    // Validate the inferred ODCS contract against the same dataset
    datakeel()
        .args([
            "check",
            odcs_path.to_str().unwrap(),
            example("populi_students.jsonl").to_str().unwrap(),
            "--format",
            "agent",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"ok\":true"));

    // Also infer compact TOML and check
    datakeel()
        .args([
            "infer",
            example("populi_students.jsonl").to_str().unwrap(),
            "--contract-format",
            "toml",
            "-o",
            toml_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    datakeel()
        .args([
            "check",
            toml_path.to_str().unwrap(),
            example("populi_students.jsonl").to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn check_cross_source_fk_and_constraints_detect_all_violations() {
    // Clean enrollments pass with --ref
    datakeel()
        .args([
            "check",
            example("canvas_enrollments.toml").to_str().unwrap(),
            example("canvas_enrollments.jsonl").to_str().unwrap(),
            "--ref",
            &format!(
                "populi_students={}",
                example("populi_students.jsonl").display()
            ),
            "--format",
            "agent",
        ])
        .assert()
        .success();

    // Bad enrollments fail with exit code 1 and compact AX JSON diagnostics
    let output = datakeel()
        .args([
            "check",
            example("canvas_enrollments.toml").to_str().unwrap(),
            example("canvas_enrollments_bad.jsonl").to_str().unwrap(),
            "--ref",
            &format!(
                "populi_students={}",
                example("populi_students.jsonl").display()
            ),
            "--format",
            "agent",
        ])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();

    let report: Value = serde_json::from_slice(&output).expect("valid JSON on stdout");
    assert_eq!(report["ok"], false);
    let codes: Vec<&str> = report["violations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"DK_ORPHAN_REFERENCE"), "codes: {codes:?}");
    assert!(codes.contains(&"DK_RANGE_VIOLATION"), "codes: {codes:?}");
    assert!(codes.contains(&"DK_NULL_VIOLATION"), "codes: {codes:?}");
}

#[test]
fn diff_catches_dropped_column_null_spike_and_row_drop() {
    let output = datakeel()
        .args([
            "diff",
            example("populi_students.jsonl").to_str().unwrap(),
            example("populi_students_today_drift.jsonl")
                .to_str()
                .unwrap(),
            "--format",
            "agent",
        ])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();

    let report: Value = serde_json::from_slice(&output).expect("valid JSON");
    assert_eq!(report["ok"], false);
    let codes: Vec<&str> = report["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["code"].as_str().unwrap())
        .collect();

    assert!(codes.contains(&"DK_DRIFT_COLUMN_DROPPED"), "codes: {codes:?}");
    assert!(codes.contains(&"DK_DRIFT_NULL_SPIKE"), "codes: {codes:?}");
    assert!(codes.contains(&"DK_DRIFT_ROW_DROP"), "codes: {codes:?}");
    assert!(codes.contains(&"DK_DRIFT_COLUMN_ADDED"), "codes: {codes:?}");
}

#[test]
fn codegen_postgres_and_pydantic() {
    datakeel()
        .args([
            "codegen",
            example("populi_students.datacontract.yaml")
                .to_str()
                .unwrap(),
            "--target",
            "postgres",
            "--schema",
            "staging",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "CREATE TABLE IF NOT EXISTS staging.populi_students",
        ))
        .stdout(predicate::str::contains("PRIMARY KEY (student_id)"))
        .stdout(predicate::str::contains("CHECK (gpa >= 0 AND gpa <= 4)"));

    datakeel()
        .args([
            "codegen",
            example("populi_students.datacontract.yaml")
                .to_str()
                .unwrap(),
            "--target",
            "pydantic",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("class PopuliStudents(BaseModel):"))
        .stdout(predicate::str::contains(
            "Literal[\"MDiv\", \"MAR\", \"MABC\", \"ThM\", \"PhD\"]",
        ))
        .stdout(predicate::str::contains("ge=0, le=4"));
}

#[test]
fn lint_valid_and_invalid_contracts() {
    datakeel()
        .args([
            "lint",
            example("populi_students.datacontract.yaml")
                .to_str()
                .unwrap(),
            "--format",
            "agent",
        ])
        .assert()
        .success();

    let dir = tempdir().unwrap();
    let bad = dir.path().join("bad.toml");
    fs::write(
        &bad,
        r#"
[contract]
name = "broken"
min_rows = 100
max_rows = 10

[columns.gpa]
type = "number"
min = 4.0
max = 0.0
pattern = "[unclosed"
references = "missing_dot"
"#,
    )
    .unwrap();

    let out = datakeel()
        .args(["lint", bad.to_str().unwrap(), "--format", "agent"])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();

    let report: Value = serde_json::from_slice(&out).unwrap();
    let codes: Vec<&str> = report["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"DK_LINT_INVALID_ROW_BOUNDS"));
    assert!(codes.contains(&"DK_LINT_INVALID_RANGE"));
    assert!(codes.contains(&"DK_LINT_INVALID_REGEX"));
    assert!(codes.contains(&"DK_LINT_INVALID_REFERENCE"));
}

#[test]
fn json_envelope_csv_stdin_and_parquet_inputs_work() {
    let dir = tempdir().unwrap();

    // 1. JSON envelope with --json-path
    let env_json = dir.path().join("api_resp.json");
    fs::write(
        &env_json,
        r#"{"meta":{"page":1},"data":{"items":[{"id":1,"score":98.5},{"id":2,"score":84.0}]}}"#,
    )
    .unwrap();

    datakeel()
        .args([
            "infer",
            env_json.to_str().unwrap(),
            "--json-path",
            "data.items",
            "--contract-format",
            "toml",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("[columns.score]"));

    // 2. CSV with leading-zero IDs preserved as strings
    let csv_path = dir.path().join("roster.csv");
    fs::write(&csv_path, "sis_id,gpa,active\n00123,3.7,true\n00456,2.9,false\n").unwrap();
    datakeel()
        .args([
            "infer",
            csv_path.to_str().unwrap(),
            "--contract-format",
            "toml",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("type = \"string\""))
        .stdout(predicate::str::contains("type = \"number\""))
        .stdout(predicate::str::contains("type = \"boolean\""));

    // 3. Stdin JSONL
    datakeel()
        .args(["infer", "-", "--contract-format", "toml", "--name", "streamed"])
        .write_stdin("{\"id\":1,\"ok\":true}\n{\"id\":2,\"ok\":false}\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("name = \"streamed\""));

    // 4. Native Parquet file read
    let pq_path = dir.path().join("students.parquet");
    let schema = Arc::new(
        parse_message_type(
            "message schema {
                REQUIRED BYTE_ARRAY student_id (UTF8);
                REQUIRED DOUBLE gpa;
            }",
        )
        .unwrap(),
    );
    let props = Arc::new(WriterProperties::builder().build());
    let file = File::create(&pq_path).unwrap();
    let mut writer = SerializedFileWriter::new(file, schema, props).unwrap();
    {
        let mut rg = writer.next_row_group().unwrap();
        {
            let mut col = rg.next_column().unwrap().unwrap();
            col.typed::<parquet::data_type::ByteArrayType>()
                .write_batch(
                    &[ByteArray::from("STU-1001"), ByteArray::from("STU-1002")],
                    None,
                    None,
                )
                .unwrap();
            col.close().unwrap();
        }
        {
            let mut col = rg.next_column().unwrap().unwrap();
            col.typed::<parquet::data_type::DoubleType>()
                .write_batch(&[3.85, 2.45], None, None)
                .unwrap();
            col.close().unwrap();
        }
        rg.close().unwrap();
    }
    writer.close().unwrap();

    datakeel()
        .args([
            "infer",
            pq_path.to_str().unwrap(),
            "--contract-format",
            "odcs",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("name: student_id"))
        .stdout(predicate::str::contains("name: gpa"));
}

#[test]
fn skill_prints_and_installs_and_errors_return_exit_2() {
    let dir = tempdir().unwrap();

    datakeel()
        .args(["skill"])
        .assert()
        .success()
        .stdout(predicate::str::contains("name: datakeel"))
        .stdout(predicate::str::contains("Exit codes:"));

    let skill_dir = dir.path().join(".agents/skills/datakeel");
    datakeel()
        .args(["skill", "--install", skill_dir.to_str().unwrap()])
        .assert()
        .success();
    assert!(skill_dir.join("SKILL.md").exists());

    // Missing file returns exit code 2 and structured JSON error envelope on stderr
    let stderr = datakeel()
        .args(["lint", "/nonexistent/contract.yaml", "--format", "agent"])
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let env: Value = serde_json::from_slice(&stderr).expect("valid JSON error envelope on stderr");
    assert_eq!(env["ok"], false);
    assert_eq!(env["error"]["code"], "DK_IO_READ");
}
