use crate::contract::{Contract, LogicalType};
use crate::data::{Dataset, Value};
use crate::error::{KeelError, Result};
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct CheckOptions {
    pub strict_override: bool,
    pub max_samples: usize,
    pub contract_path: String,
    pub data_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct SampleFailure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<usize>,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    pub code: &'static str,
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    pub count: usize,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<SampleFailure>,
    pub fix_hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix_command: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckReport {
    pub schema_version: &'static str,
    pub command: &'static str,
    pub passed: bool,
    pub contract_name: String,
    pub contract_path: String,
    pub data_path: String,
    pub rows_checked: usize,
    pub columns_checked: usize,
    pub violation_count: usize,
    pub total_failing_cells: usize,
    pub violations: Vec<Violation>,
}

pub fn check_dataset(
    contract: &Contract,
    dataset: &Dataset,
    options: &CheckOptions,
    ref_datasets: &HashMap<String, Dataset>,
) -> Result<CheckReport> {
    let mut violations: Vec<Violation> = Vec::new();
    let row_count = dataset.row_count();
    let strict = options.strict_override || contract.strict;

    // 1. Table row-count bounds
    if let Some(min_rows) = contract.min_rows
        && row_count < min_rows
    {
        violations.push(Violation {
            code: "DK_ROW_COUNT_LOW",
            severity: Severity::Error,
            column: None,
            count: 1,
            message: format!(
                "Dataset has {row_count} rows, below contract minimum of {min_rows}."
            ),
            samples: vec![],
            fix_hint: "Check upstream API pagination, auth expiration, or date-window filters before loading into Postgres.".to_string(),
            fix_command: None,
        });
    }
    if let Some(max_rows) = contract.max_rows
        && row_count > max_rows
    {
        violations.push(Violation {
            code: "DK_ROW_COUNT_HIGH",
            severity: Severity::Error,
            column: None,
            count: 1,
            message: format!(
                "Dataset has {row_count} rows, above contract maximum of {max_rows}."
            ),
            samples: vec![],
            fix_hint: "Check for duplicate extraction loops or missing deduplication.".to_string(),
            fix_command: None,
        });
    }

    // 2. Strict mode: unexpected columns
    if strict {
        let extra: Vec<String> = dataset
            .columns
            .iter()
            .filter(|c| !contract.columns.contains_key(*c))
            .cloned()
            .collect();
        if !extra.is_empty() {
            let samples = extra
                .iter()
                .take(options.max_samples)
                .map(|c| SampleFailure {
                    row: None,
                    value: c.clone(),
                })
                .collect();
            violations.push(Violation {
                code: "DK_EXTRA_COLUMN",
                severity: Severity::Error,
                column: None,
                count: extra.len(),
                message: format!(
                    "Found {} unexpected column(s) in strict mode: {}",
                    extra.len(),
                    extra.join(", ")
                ),
                samples,
                fix_hint: "Add the new column(s) to the contract or drop them in the ETL projection step.".to_string(),
                fix_command: Some(format!(
                    "datakeel diff {} {}",
                    options.contract_path, options.data_path
                )),
            });
        }
    }

    // 3. Column-by-column validation
    for (col_name, col_spec) in &contract.columns {
        let col_present = dataset.columns.contains(col_name);
        if !col_present {
            if col_spec.required {
                violations.push(Violation {
                    code: "DK_MISSING_COLUMN",
                    severity: Severity::Error,
                    column: Some(col_name.clone()),
                    count: row_count.max(1),
                    message: format!("Required column '{col_name}' is missing from dataset."),
                    samples: vec![],
                    fix_hint: format!(
                        "Verify upstream extractor includes '{col_name}' or check if the API renamed the field."
                    ),
                    fix_command: Some(format!(
                        "datakeel diff {} {}",
                        options.contract_path, options.data_path
                    )),
                });
            }
            continue;
        }

        let compiled_regex = if let Some(ref pat) = col_spec.pattern {
            Some(Regex::new(pat).map_err(|e| KeelError::InvalidRegex {
                column: col_name.clone(),
                pattern: pat.clone(),
                detail: e.to_string(),
            })?)
        } else {
            None
        };

        let allowed_set: Option<HashSet<&str>> = col_spec
            .allowed_values
            .as_ref()
            .map(|vals| vals.iter().map(String::as_str).collect());

        // Pre-index reference column if `references` is set.
        let ref_index: Option<(String, String, HashSet<String>)> = if let Some(ref fk) =
            col_spec.references
        {
            if let Some((ref_table, ref_col)) = fk.split_once('.') {
                if let Some(ref_ds) = ref_datasets.get(ref_table) {
                    let mut keys = HashSet::new();
                    for r in &ref_ds.rows {
                        if let Some(v) = r.get(ref_col)
                            && let Some(k) = v.canonical_key()
                        {
                            keys.insert(k);
                        }
                    }
                    Some((ref_table.to_string(), ref_col.to_string(), keys))
                } else {
                    violations.push(Violation {
                        code: "DK_MISSING_REF_DATASET",
                        severity: Severity::Warning,
                        column: Some(col_name.clone()),
                        count: 1,
                        message: format!(
                            "Column '{col_name}' references '{fk}', but no `--ref {ref_table}=<path>` dataset was provided (skipped FK check)."
                        ),
                        samples: vec![],
                        fix_hint: format!(
                            "Pass `--ref {ref_table}=<path>` to enable cross-source referential integrity checks."
                        ),
                        fix_command: Some(format!(
                            "datakeel check {} {} --ref {}=<path>",
                            options.contract_path, options.data_path, ref_table
                        )),
                    });
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let mut null_count = 0usize;
        let mut null_samples: Vec<SampleFailure> = Vec::new();

        let mut type_mismatch_count = 0usize;
        let mut type_samples: Vec<SampleFailure> = Vec::new();

        let mut range_count = 0usize;
        let mut range_samples: Vec<SampleFailure> = Vec::new();

        let mut length_count = 0usize;
        let mut length_samples: Vec<SampleFailure> = Vec::new();

        let mut pattern_count = 0usize;
        let mut pattern_samples: Vec<SampleFailure> = Vec::new();

        let mut enum_count = 0usize;
        let mut enum_samples: Vec<SampleFailure> = Vec::new();

        let mut seen_unique: HashSet<String> = HashSet::new();
        let mut dup_count = 0usize;
        let mut dup_samples: Vec<SampleFailure> = Vec::new();

        let mut orphan_count = 0usize;
        let mut orphan_samples: Vec<SampleFailure> = Vec::new();

        for (idx, row) in dataset.rows.iter().enumerate() {
            let row_num = idx + 1;
            let val = row.get(col_name).unwrap_or(&Value::Null);

            if val.is_null() {
                null_count += 1;
                if null_samples.len() < options.max_samples {
                    null_samples.push(SampleFailure {
                        row: Some(row_num),
                        value: "null".to_string(),
                    });
                }
                continue;
            }

            // Type check
            if let Some(actual_type) = val.logical_type()
                && !LogicalType::is_compatible_with(actual_type, col_spec.logical_type)
            {
                type_mismatch_count += 1;
                if type_samples.len() < options.max_samples {
                    type_samples.push(SampleFailure {
                        row: Some(row_num),
                        value: format!("{val} ({actual_type})"),
                    });
                }
                continue;
            }

            // Numeric range check
            if (col_spec.min.is_some() || col_spec.max.is_some())
                && let Some(num) = val.as_f64()
            {
                let below = col_spec.min.is_some_and(|m| num < m);
                let above = col_spec.max.is_some_and(|m| num > m);
                if below || above {
                    range_count += 1;
                    if range_samples.len() < options.max_samples {
                        range_samples.push(SampleFailure {
                            row: Some(row_num),
                            value: val.to_string(),
                        });
                    }
                }
            }

            // String length, pattern, enum checks
            if let Some(s) = val.as_str() {
                let char_len = s.chars().count();
                if col_spec.min_length.is_some_and(|m| char_len < m)
                    || col_spec.max_length.is_some_and(|m| char_len > m)
                {
                    length_count += 1;
                    if length_samples.len() < options.max_samples {
                        length_samples.push(SampleFailure {
                            row: Some(row_num),
                            value: format!("{s:?} (len={char_len})"),
                        });
                    }
                }

                if let Some(ref re) = compiled_regex
                    && !re.is_match(s)
                {
                    pattern_count += 1;
                    if pattern_samples.len() < options.max_samples {
                        pattern_samples.push(SampleFailure {
                            row: Some(row_num),
                            value: s.to_string(),
                        });
                    }
                }

                if let Some(ref allowed) = allowed_set
                    && !allowed.contains(s)
                {
                    enum_count += 1;
                    if enum_samples.len() < options.max_samples {
                        enum_samples.push(SampleFailure {
                            row: Some(row_num),
                            value: s.to_string(),
                        });
                    }
                }
            }

            // Uniqueness / PK check
            if (col_spec.unique || col_spec.primary_key)
                && let Some(key) = val.canonical_key()
                && !seen_unique.insert(key.clone())
            {
                dup_count += 1;
                if dup_samples.len() < options.max_samples {
                    dup_samples.push(SampleFailure {
                        row: Some(row_num),
                        value: key,
                    });
                }
            }

            // Cross-source FK check
            if let Some((_, _, ref ref_keys)) = ref_index
                && let Some(key) = val.canonical_key()
                && !ref_keys.contains(&key)
            {
                orphan_count += 1;
                if orphan_samples.len() < options.max_samples {
                    orphan_samples.push(SampleFailure {
                        row: Some(row_num),
                        value: key,
                    });
                }
            }
        }

        // Emit deduplicated column violations
        if col_spec.required && null_count > 0 {
            violations.push(Violation {
                code: "DK_NULL_VIOLATION",
                severity: Severity::Error,
                column: Some(col_name.clone()),
                count: null_count,
                message: format!(
                    "Required column '{col_name}' contains {null_count} null value(s) out of {row_count} rows."
                ),
                samples: null_samples,
                fix_hint: format!(
                    "Filter/quarantine null '{col_name}' records in ETL, backfill a default, or set `required: false` with a `max_null_rate` budget."
                ),
                fix_command: None,
            });
        } else if let Some(max_rate) = col_spec.max_null_rate
            && row_count > 0
        {
            let actual_rate = null_count as f64 / row_count as f64;
            if actual_rate > max_rate {
                violations.push(Violation {
                    code: "DK_NULL_RATE_EXCEEDED",
                    severity: Severity::Error,
                    column: Some(col_name.clone()),
                    count: null_count,
                    message: format!(
                        "Column '{col_name}' null rate is {:.1}% ({null_count}/{row_count}), exceeding budget of {:.1}%.",
                        actual_rate * 100.0,
                        max_rate * 100.0
                    ),
                    samples: null_samples,
                    fix_hint: format!(
                        "Investigate upstream null spike on '{col_name}' before overwriting downstream analytics."
                    ),
                    fix_command: None,
                });
            }
        }

        if type_mismatch_count > 0 {
            violations.push(Violation {
                code: "DK_TYPE_MISMATCH",
                severity: Severity::Error,
                column: Some(col_name.clone()),
                count: type_mismatch_count,
                message: format!(
                    "Column '{col_name}' expects type '{}', but {type_mismatch_count} row(s) have an incompatible type.",
                    col_spec.logical_type
                ),
                samples: type_samples,
                fix_hint: format!(
                    "Coerce or reject malformed '{col_name}' values during extraction, or widen the contract type if intentional."
                ),
                fix_command: None,
            });
        }

        if range_count > 0 {
            let bounds = match (col_spec.min, col_spec.max) {
                (Some(lo), Some(hi)) => format!("[{lo}, {hi}]"),
                (Some(lo), None) => format!(">= {lo}"),
                (None, Some(hi)) => format!("<= {hi}"),
                (None, None) => String::new(),
            };
            violations.push(Violation {
                code: "DK_RANGE_VIOLATION",
                severity: Severity::Error,
                column: Some(col_name.clone()),
                count: range_count,
                message: format!(
                    "Column '{col_name}' has {range_count} value(s) outside allowed range {bounds}."
                ),
                samples: range_samples,
                fix_hint: format!(
                    "Clamp, quarantine, or normalize out-of-range '{col_name}' values (e.g. bad grade/percentage units) before loading."
                ),
                fix_command: None,
            });
        }

        if length_count > 0 {
            violations.push(Violation {
                code: "DK_LENGTH_VIOLATION",
                severity: Severity::Error,
                column: Some(col_name.clone()),
                count: length_count,
                message: format!(
                    "Column '{col_name}' has {length_count} value(s) violating string length bounds (min={:?}, max={:?}).",
                    col_spec.min_length, col_spec.max_length
                ),
                samples: length_samples,
                fix_hint: format!("Trim or reject invalid-length '{col_name}' values."),
                fix_command: None,
            });
        }

        if pattern_count > 0 {
            violations.push(Violation {
                code: "DK_PATTERN_MISMATCH",
                severity: Severity::Error,
                column: Some(col_name.clone()),
                count: pattern_count,
                message: format!(
                    "Column '{col_name}' has {pattern_count} value(s) failing regex `{}`.",
                    col_spec.pattern.as_deref().unwrap_or("")
                ),
                samples: pattern_samples,
                fix_hint: format!(
                    "Normalize formatting on '{col_name}' or verify the regex pattern in the contract."
                ),
                fix_command: None,
            });
        }

        if enum_count > 0 {
            let allowed_preview = col_spec
                .allowed_values
                .as_ref()
                .map(|v| v.join(", "))
                .unwrap_or_default();
            violations.push(Violation {
                code: "DK_ENUM_VIOLATION",
                severity: Severity::Error,
                column: Some(col_name.clone()),
                count: enum_count,
                message: format!(
                    "Column '{col_name}' has {enum_count} value(s) outside allowed enum [{allowed_preview}]."
                ),
                samples: enum_samples,
                fix_hint: format!(
                    "Map unexpected '{col_name}' categories in ETL or add the new valid enum value to the contract."
                ),
                fix_command: None,
            });
        }

        if dup_count > 0 {
            violations.push(Violation {
                code: "DK_DUPLICATE_KEY",
                severity: Severity::Error,
                column: Some(col_name.clone()),
                count: dup_count,
                message: format!(
                    "Unique column '{col_name}' has {dup_count} duplicate value(s)."
                ),
                samples: dup_samples,
                fix_hint: format!(
                    "Deduplicate by '{col_name}' (keeping latest updated_at) before Postgres UPSERT."
                ),
                fix_command: None,
            });
        }

        if orphan_count > 0 {
            let fk = col_spec.references.as_deref().unwrap_or("");
            violations.push(Violation {
                code: "DK_ORPHAN_REFERENCE",
                severity: Severity::Error,
                column: Some(col_name.clone()),
                count: orphan_count,
                message: format!(
                    "Column '{col_name}' has {orphan_count} orphan foreign-key value(s) not present in '{fk}'."
                ),
                samples: orphan_samples,
                fix_hint: format!(
                    "Reconcile missing IDs between sources (e.g., sync '{fk}' first or quarantine unmatched records so students are not silently dropped in JOINs)."
                ),
                fix_command: None,
            });
        }
    }

    let error_count = violations
        .iter()
        .filter(|v| v.severity == Severity::Error)
        .count();
    let total_failing_cells = violations
        .iter()
        .filter(|v| v.severity == Severity::Error)
        .map(|v| v.count)
        .sum();

    Ok(CheckReport {
        schema_version: "1.0",
        command: "check",
        passed: error_count == 0,
        contract_name: contract.name.clone(),
        contract_path: options.contract_path.clone(),
        data_path: options.data_path.clone(),
        rows_checked: row_count,
        columns_checked: contract.columns.len(),
        violation_count: violations.len(),
        total_failing_cells,
        violations,
    })
}
