use crate::engine::check::CheckReport;
use crate::engine::diff::DiffReport;
use crate::engine::lint::LintReport;
use crate::error::{KeelError, Result};
use serde::Serialize;

/// Token-budgeted, deterministic agent-facing summary envelope (`--format agent`).
#[derive(Debug, Serialize)]
struct AgentCheckEnvelope<'a> {
    schema_version: &'static str,
    command: &'static str,
    ok: bool,
    summary: String,
    rows_checked: usize,
    columns_checked: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    violations: &'a Vec<crate::engine::check::Violation>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    next_actions: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AgentDiffEnvelope<'a> {
    schema_version: &'static str,
    command: &'static str,
    ok: bool,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline_rows: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_rows: Option<usize>,
    breaking_count: usize,
    warning_count: usize,
    info_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    changes: &'a Vec<crate::engine::diff::DriftChange>,
}

#[derive(Debug, Serialize)]
struct AgentLintEnvelope<'a> {
    schema_version: &'static str,
    command: &'static str,
    ok: bool,
    summary: String,
    columns_defined: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    issues: &'a Vec<crate::engine::lint::LintIssue>,
}

pub fn print_check_report(report: &CheckReport) -> Result<()> {
    let summary = if report.passed {
        format!(
            "PASS: '{}' validated {} rows across {} columns with 0 errors.",
            report.contract_name, report.rows_checked, report.columns_checked
        )
    } else {
        format!(
            "FAIL: '{}' found {} violation(s) ({} failing cells) in {} rows.",
            report.contract_name,
            report.violation_count,
            report.total_failing_cells,
            report.rows_checked
        )
    };

    let mut next_actions: Vec<String> = Vec::new();
    for v in &report.violations {
        if let Some(ref cmd) = v.fix_command
            && !next_actions.contains(cmd)
        {
            next_actions.push(cmd.clone());
        }
    }

    let env = AgentCheckEnvelope {
        schema_version: "1.0",
        command: "check",
        ok: report.passed,
        summary,
        rows_checked: report.rows_checked,
        columns_checked: report.columns_checked,
        violations: &report.violations,
        next_actions,
    };

    let json = serde_json::to_string(&env)
        .map_err(|e| KeelError::General(format!("JSON serialization error: {e}")))?;
    println!("{json}");
    Ok(())
}

pub fn print_diff_report(report: &DiffReport) -> Result<()> {
    let summary = if report.failed {
        format!(
            "DRIFT: {} breaking, {} warning, {} info change(s) between {} and {}.",
            report.breaking_count,
            report.warning_count,
            report.info_count,
            report.baseline_path,
            report.current_path
        )
    } else if report.changes.is_empty() {
        format!(
            "CLEAN: no schema or distribution drift between {} and {}.",
            report.baseline_path, report.current_path
        )
    } else {
        format!(
            "COMPATIBLE: {} non-breaking change(s) between {} and {}.",
            report.changes.len(),
            report.baseline_path,
            report.current_path
        )
    };

    let env = AgentDiffEnvelope {
        schema_version: "1.0",
        command: "diff",
        ok: !report.failed,
        summary,
        baseline_rows: report.baseline_rows,
        current_rows: report.current_rows,
        breaking_count: report.breaking_count,
        warning_count: report.warning_count,
        info_count: report.info_count,
        changes: &report.changes,
    };

    let json = serde_json::to_string(&env)
        .map_err(|e| KeelError::General(format!("JSON serialization error: {e}")))?;
    println!("{json}");
    Ok(())
}

pub fn print_lint_report(report: &LintReport) -> Result<()> {
    let summary = if report.valid {
        format!(
            "VALID: contract '{}' ({} columns) passed lint.",
            report.contract_name, report.columns_defined
        )
    } else {
        format!(
            "INVALID: contract '{}' has {} lint issue(s).",
            report.contract_name,
            report.issues.len()
        )
    };

    let env = AgentLintEnvelope {
        schema_version: "1.0",
        command: "lint",
        ok: report.valid,
        summary,
        columns_defined: report.columns_defined,
        issues: &report.issues,
    };

    let json = serde_json::to_string(&env)
        .map_err(|e| KeelError::General(format!("JSON serialization error: {e}")))?;
    println!("{json}");
    Ok(())
}
