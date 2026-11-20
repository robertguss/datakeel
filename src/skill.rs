use crate::error::{KeelError, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub const SKILL_MD: &str = r#"---
name: datakeel
description: Validate, infer, diff, and generate Postgres DDL / Pydantic v2 models from ODCS v3 YAML or datakeel.toml data contracts. Use before writing ETL loaders, SQL views, or database migrations so you work from verified ground-truth schemas instead of guessing raw API payloads.
---

# DataKeel Agent Manual

## Identity & Scope

`datakeel` is a single-binary Rust CLI for **data contracts, schema drift detection, cross-source referential integrity, and Postgres/Pydantic codegen**. It natively supports **Open Data Contract Standard (ODCS v3.2.0) YAML** (`*.yaml` / `*.yml`) and compact **DataKeel TOML** (`*.toml`), and reads **JSON, JSONL/NDJSON, CSV, TSV, Parquet, and live PostgreSQL tables** (`pg://[schema.]table` or `postgres://...`).

## Output & Exit Code Contract

- **TTY-aware dual mode:** Automatically outputs compact single-line JSON (`--format agent`) when piped/non-TTY and human-readable output in an interactive terminal. You can also pass `--format agent` or `--format json` explicitly.
- **Strict stream separation:** `stdout` contains ONLY data payloads or generated contracts/code. `stderr` receives runtime/config errors (`ErrorEnvelope` JSON when non-TTY).
- **Zero interactivity & secret hygiene:** Never prompts for input. Prefer `pg://schema.table` with `DATABASE_URL` (or `DATAKEEL_DATABASE_URL`) in the environment instead of embedding passwords on the command line; `datakeel` redacts URI passwords in all reports and errors.
- **Exit codes:**
  - `0` — Clean pass / no breaking drift / contract valid
  - `1` — Data contract violation, breaking drift, or invalid contract lint
  - `2` — CLI usage, IO, Postgres connection/query, or parse error (check `stderr` JSON `error.code` and `error.fix_hint`)

## Core Command Matrix

| Goal | Command |
| :--- | :--- |
| Infer ODCS v3 contract from raw API dump | `datakeel infer extract.jsonl -o contracts/table.yaml` |
| Infer contract directly from live Postgres | `datakeel infer pg://staging.populi_students --limit 10000 -o contracts/populi_students.yaml` |
| Infer compact TOML contract with enums | `datakeel infer extract.csv --contract-format toml --infer-enums 10 -o contracts/table.toml` |
| Infer from nested JSON API envelope | `datakeel infer response.json --json-path data.records -o contracts/table.yaml` |
| Validate data against a contract | `datakeel check contracts/table.yaml extract.jsonl --format agent` |
| Validate a live Postgres table slice | `datakeel check contracts/table.yaml pg://staging.populi_students --where "status = 'active'" --format agent` |
| Validate local extract vs live Postgres FK | `datakeel check contracts/enrollments.toml enrollments.jsonl --ref populi_students=pg://staging.populi_students --format agent` |
| Split clean & quarantined rows with budget | `datakeel check contracts/enrollments.toml enrollments.jsonl --clean good.jsonl --quarantine bad.jsonl --max-quarantine-rate 0.05 --format agent` |
| Detect drift vs live Postgres before load | `datakeel diff pg://staging.populi_students today_populi.jsonl --format agent` |
| Generate Postgres DDL from contract | `datakeel codegen contracts/table.yaml --target postgres --schema staging` |
| Generate Python Pydantic v2 model | `datakeel codegen contracts/table.yaml --target pydantic` |
| Lint a contract file before committing | `datakeel lint contracts/table.yaml --format agent` |

## Agent Decision Tree for ETL Pipelines

1. **Before writing ETL code for a new API source:**
   - Do **not** read raw 50MB JSON dumps into context.
   - Run `datakeel infer <sample_file> -o contracts/<source>.yaml` and read the ~30-line contract instead.
2. **Keep Python models and Postgres tables 100% synchronized:**
   - Run `datakeel codegen contracts/<source>.yaml --target pydantic` and `--target postgres` rather than typing column names from memory.
3. **Gate every extraction before Postgres load:**
   - Run `datakeel check contracts/<source>.yaml <extract> --format agent` (add `--ref <parent>=<file>` for foreign keys).
   - If exit code is `1`, inspect each violation's `code`, `column`, `count`, `samples` (capped at 3 by default), and `fix_hint` to fix the extractor or quarantine bad rows.
4. **Catch silent upstream API breakage:**
   - Run `datakeel diff <previous_extract> <current_extract> --format agent`.
   - Exit code `1` indicates breaking drift (`DK_DRIFT_COLUMN_DROPPED`, `DK_DRIFT_TYPE_INCOMPATIBLE`, `DK_DRIFT_NULL_SPIKE`, `DK_DRIFT_ROW_DROP`). Halt load and inspect before overwriting downstream tables.

## Gotchas & Anti-Patterns

- **Do not** parse human-formatted output; always let non-TTY auto-mode select `agent` or pass `--format agent` / `--format json`.
- **Do not** ignore `DK_MISSING_REF_DATASET` warnings if a contract declares `references`; pass `--ref <dataset_name>=<file_path>` to verify foreign keys so orphaned records are not silently dropped by SQL `INNER JOIN`s.
- For JSON API responses wrapped in an object (e.g. `{"items": [...]}`), pass `--json-path items`.
- Stdin (`-`) defaults to JSONL; pass `--input-format json` or `--input-format csv` if piping another format. Parquet requires a file path.
"#;

pub fn install_skill(target: &Path) -> Result<PathBuf> {
    let dest = if target
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
    {
        target.to_path_buf()
    } else {
        target.join("SKILL.md")
    };

    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| KeelError::IoWrite {
            path: dest.clone(),
            source,
        })?;
    }

    fs::write(&dest, SKILL_MD).map_err(|source| KeelError::IoWrite {
        path: dest.clone(),
        source,
    })?;

    Ok(dest)
}
