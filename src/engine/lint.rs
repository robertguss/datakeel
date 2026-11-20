use crate::contract::Contract;
use crate::engine::check::Severity;
use crate::error::Result;
use regex::Regex;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct LintIssue {
    pub code: &'static str,
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    pub message: String,
    pub fix_hint: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LintReport {
    pub schema_version: &'static str,
    pub command: &'static str,
    pub valid: bool,
    pub contract_path: String,
    pub contract_name: String,
    pub columns_defined: usize,
    pub issues: Vec<LintIssue>,
}

pub fn lint_contract_file(path: &Path) -> Result<LintReport> {
    let contract = Contract::load_from_path(path)?;
    let mut issues = Vec::new();

    if contract.name.trim().is_empty() {
        issues.push(LintIssue {
            code: "DK_LINT_EMPTY_NAME",
            severity: Severity::Error,
            column: None,
            message: "Contract `name` cannot be empty.".to_string(),
            fix_hint: "Set a non-empty snake_case contract name.".to_string(),
        });
    }

    if contract.columns.is_empty() {
        issues.push(LintIssue {
            code: "DK_LINT_NO_COLUMNS",
            severity: Severity::Error,
            column: None,
            message: "Contract defines zero columns.".to_string(),
            fix_hint: "Define at least one column or run `datakeel infer <data>` to generate a baseline contract.".to_string(),
        });
    }

    if let (Some(lo), Some(hi)) = (contract.min_rows, contract.max_rows)
        && lo > hi
    {
        issues.push(LintIssue {
            code: "DK_LINT_INVALID_ROW_BOUNDS",
            severity: Severity::Error,
            column: None,
            message: format!("min_rows ({lo}) is greater than max_rows ({hi})."),
            fix_hint: "Ensure min_rows <= max_rows.".to_string(),
        });
    }

    for (col_name, col) in &contract.columns {
        if let (Some(lo), Some(hi)) = (col.min, col.max)
            && lo > hi
        {
            issues.push(LintIssue {
                code: "DK_LINT_INVALID_RANGE",
                severity: Severity::Error,
                column: Some(col_name.clone()),
                message: format!("Column '{col_name}' has min ({lo}) > max ({hi})."),
                fix_hint: format!("Ensure `min <= max` on column '{col_name}'."),
            });
        }

        if let (Some(lo), Some(hi)) = (col.min_length, col.max_length)
            && lo > hi
        {
            issues.push(LintIssue {
                code: "DK_LINT_INVALID_LENGTH",
                severity: Severity::Error,
                column: Some(col_name.clone()),
                message: format!(
                    "Column '{col_name}' has min_length ({lo}) > max_length ({hi})."
                ),
                fix_hint: format!("Ensure `min_length <= max_length` on column '{col_name}'."),
            });
        }

        if let Some(rate) = col.max_null_rate
            && !(0.0..=1.0).contains(&rate)
        {
            issues.push(LintIssue {
                code: "DK_LINT_INVALID_NULL_RATE",
                severity: Severity::Error,
                column: Some(col_name.clone()),
                message: format!(
                    "Column '{col_name}' has max_null_rate ({rate}) outside [0.0, 1.0]."
                ),
                fix_hint: "Express max_null_rate as a fraction between 0.0 and 1.0 (e.g. 0.05 for 5%).".to_string(),
            });
        }

        if let Some(ref pat) = col.pattern
            && let Err(e) = Regex::new(pat)
        {
            issues.push(LintIssue {
                code: "DK_LINT_INVALID_REGEX",
                severity: Severity::Error,
                column: Some(col_name.clone()),
                message: format!("Column '{col_name}' has invalid regex `{pat}`: {e}"),
                fix_hint: format!("Fix the regular expression syntax on column '{col_name}'."),
            });
        }

        if let Some(ref fk) = col.references {
            let valid_fk = fk
                .split_once('.')
                .is_some_and(|(t, c)| !t.trim().is_empty() && !c.trim().is_empty() && !c.contains('.'));
            if !valid_fk {
                issues.push(LintIssue {
                    code: "DK_LINT_INVALID_REFERENCE",
                    severity: Severity::Error,
                    column: Some(col_name.clone()),
                    message: format!(
                        "Column '{col_name}' has invalid `references` value '{fk}' (expected `dataset.column`)."
                    ),
                    fix_hint: "Format `references` as `<ref_name>.<column_name>`, e.g. `populi_students.student_id`.".to_string(),
                });
            }
        }

        if let Some(ref vals) = col.allowed_values
            && vals.is_empty()
        {
            issues.push(LintIssue {
                code: "DK_LINT_EMPTY_ENUM",
                severity: Severity::Warning,
                column: Some(col_name.clone()),
                message: format!("Column '{col_name}' has an empty enum list."),
                fix_hint: "Remove the empty `enum` constraint or list allowed values.".to_string(),
            });
        }
    }

    let has_errors = issues.iter().any(|i| i.severity == Severity::Error);
    Ok(LintReport {
        schema_version: "1.0",
        command: "lint",
        valid: !has_errors,
        contract_path: path.display().to_string(),
        contract_name: contract.name,
        columns_defined: contract.columns.len(),
        issues,
    })
}
