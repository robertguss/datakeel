use crate::contract::{Contract, LogicalType};
use crate::data::{Dataset, Row, Value};
use crate::error::{KeelError, Result};
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CheckOptions {
    pub strict_override: bool,
    pub max_samples: usize,
    pub contract_path: String,
    pub data_path: String,
    pub clean_path: Option<PathBuf>,
    pub quarantine_path: Option<PathBuf>,
    pub max_quarantine_rate: Option<f64>,
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
pub struct RowError {
    pub code: &'static str,
    pub column: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
struct QuarantinedRecord<'a> {
    row: usize,
    errors: &'a [RowError],
    record: &'a Row,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuarantineSummary {
    pub clean_rows: usize,
    pub quarantined_rows: usize,
    pub quarantine_rate: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_quarantine_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clean_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quarantine_path: Option<String>,
    pub within_budget: bool,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quarantine: Option<QuarantineSummary>,
    pub violations: Vec<Violation>,
}

pub fn check_dataset(
    contract: &Contract,
    dataset: &Dataset,
    options: &CheckOptions,
    ref_datasets: &HashMap<String, Dataset>,
) -> Result<CheckReport> {
    if let Some(rate) = options.max_quarantine_rate
        && !(0.0..=1.0).contains(&rate)
    {
        return Err(KeelError::General(format!(
            "`--max-quarantine-rate` must be between 0.0 and 1.0, got {rate}"
        )));
    }

    let mut violations: Vec<Violation> = Vec::new();
    let mut has_fatal_table_error = false;
    let row_count = dataset.row_count();
    let strict = options.strict_override || contract.strict;
    let track_rows = options.clean_path.is_some()
        || options.quarantine_path.is_some()
        || options.max_quarantine_rate.is_some();
    let mut row_errors: Vec<Vec<RowError>> = if track_rows {
        vec![Vec::new(); row_count]
    } else {
        Vec::new()
    };

    // 1. Table row-count bounds
    if let Some(min_rows) = contract.min_rows
        && row_count < min_rows
    {
        has_fatal_table_error = true;
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
        has_fatal_table_error = true;
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
            has_fatal_table_error = true;
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
                has_fatal_table_error = true;
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
                if track_rows && col_spec.required {
                    row_errors[idx].push(RowError {
                        code: "DK_NULL_VIOLATION",
                        column: col_name.clone(),
                        message: format!("Required column '{col_name}' is null."),
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
                if track_rows {
                    row_errors[idx].push(RowError {
                        code: "DK_TYPE_MISMATCH",
                        column: col_name.clone(),
                        message: format!(
                            "Expected '{}', got '{actual_type}' ({val}).",
                            col_spec.logical_type
                        ),
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
                    if track_rows {
                        row_errors[idx].push(RowError {
                            code: "DK_RANGE_VIOLATION",
                            column: col_name.clone(),
                            message: format!(
                                "Value {val} outside allowed range (min={:?}, max={:?}).",
                                col_spec.min, col_spec.max
                            ),
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
                    if track_rows {
                        row_errors[idx].push(RowError {
                            code: "DK_LENGTH_VIOLATION",
                            column: col_name.clone(),
                            message: format!(
                                "String length {char_len} outside bounds (min={:?}, max={:?}).",
                                col_spec.min_length, col_spec.max_length
                            ),
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
                    if track_rows {
                        row_errors[idx].push(RowError {
                            code: "DK_PATTERN_MISMATCH",
                            column: col_name.clone(),
                            message: format!(
                                "Value '{s}' does not match pattern `{}`.",
                                col_spec.pattern.as_deref().unwrap_or("")
                            ),
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
                    if track_rows {
                        row_errors[idx].push(RowError {
                            code: "DK_ENUM_VIOLATION",
                            column: col_name.clone(),
                            message: format!("Value '{s}' is not in allowed enum."),
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
                        value: key.clone(),
                    });
                }
                if track_rows {
                    row_errors[idx].push(RowError {
                        code: "DK_DUPLICATE_KEY",
                        column: col_name.clone(),
                        message: format!("Duplicate value '{key}' on unique column '{col_name}'."),
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
                        value: key.clone(),
                    });
                }
                if track_rows {
                    let fk = col_spec.references.as_deref().unwrap_or("");
                    row_errors[idx].push(RowError {
                        code: "DK_ORPHAN_REFERENCE",
                        column: col_name.clone(),
                        message: format!("Foreign key '{key}' not found in '{fk}'."),
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
                has_fatal_table_error = true;
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

    // 4. Write clean & quarantined row splits if requested
    let quarantine_summary = if track_rows {
        let mut clean_writer = match options.clean_path {
            Some(ref p) => Some((p.clone(), create_buf_writer(p)?)),
            None => None,
        };
        let mut quarantine_writer = match options.quarantine_path {
            Some(ref p) => Some((p.clone(), create_buf_writer(p)?)),
            None => None,
        };

        let mut clean_rows = 0usize;
        let mut quarantined_rows = 0usize;

        for (idx, row) in dataset.rows.iter().enumerate() {
            let errs = &row_errors[idx];
            if errs.is_empty() {
                clean_rows += 1;
                if let Some((ref path, ref mut w)) = clean_writer {
                    serde_json::to_writer(&mut *w, row).map_err(|e| {
                        KeelError::General(format!("JSON encode error: {e}"))
                    })?;
                    w.write_all(b"\n").map_err(|source| KeelError::IoWrite {
                        path: path.clone(),
                        source,
                    })?;
                }
            } else {
                quarantined_rows += 1;
                if let Some((ref path, ref mut w)) = quarantine_writer {
                    let wrapped = QuarantinedRecord {
                        row: idx + 1,
                        errors: errs,
                        record: row,
                    };
                    serde_json::to_writer(&mut *w, &wrapped).map_err(|e| {
                        KeelError::General(format!("JSON encode error: {e}"))
                    })?;
                    w.write_all(b"\n").map_err(|source| KeelError::IoWrite {
                        path: path.clone(),
                        source,
                    })?;
                }
            }
        }

        if let Some((path, mut w)) = clean_writer {
            w.flush()
                .map_err(|source| KeelError::IoWrite { path, source })?;
        }
        if let Some((path, mut w)) = quarantine_writer {
            w.flush()
                .map_err(|source| KeelError::IoWrite { path, source })?;
        }

        let quarantine_rate = if row_count == 0 {
            0.0
        } else {
            quarantined_rows as f64 / row_count as f64
        };

        let within_budget = match options.max_quarantine_rate {
            Some(limit) => !has_fatal_table_error && quarantine_rate <= limit,
            None => quarantined_rows == 0 && !has_fatal_table_error,
        };

        if let Some(limit) = options.max_quarantine_rate
            && quarantine_rate > limit
        {
            violations.push(Violation {
                code: "DK_QUARANTINE_RATE_EXCEEDED",
                severity: Severity::Error,
                column: None,
                count: quarantined_rows,
                message: format!(
                    "Quarantined {:.1}% of rows ({quarantined_rows}/{row_count}), exceeding `--max-quarantine-rate` budget of {:.1}%.",
                    quarantine_rate * 100.0,
                    limit * 100.0
                ),
                samples: vec![],
                fix_hint: "Inspect the quarantine JSONL file for systemic upstream ETL issues before loading.".to_string(),
                fix_command: None,
            });
        }

        Some(QuarantineSummary {
            clean_rows,
            quarantined_rows,
            quarantine_rate,
            max_quarantine_rate: options.max_quarantine_rate,
            clean_path: options
                .clean_path
                .as_ref()
                .map(|p| p.display().to_string()),
            quarantine_path: options
                .quarantine_path
                .as_ref()
                .map(|p| p.display().to_string()),
            within_budget,
        })
    } else {
        None
    };

    let error_count = violations
        .iter()
        .filter(|v| v.severity == Severity::Error)
        .count();
    let total_failing_cells = violations
        .iter()
        .filter(|v| v.severity == Severity::Error)
        .map(|v| v.count)
        .sum();

    let passed = if options.max_quarantine_rate.is_some() {
        quarantine_summary
            .as_ref()
            .is_some_and(|q| q.within_budget)
    } else {
        error_count == 0
    };

    Ok(CheckReport {
        schema_version: "1.0",
        command: "check",
        passed,
        contract_name: contract.name.clone(),
        contract_path: options.contract_path.clone(),
        data_path: options.data_path.clone(),
        rows_checked: row_count,
        columns_checked: contract.columns.len(),
        violation_count: violations.len(),
        total_failing_cells,
        quarantine: quarantine_summary,
        violations,
    })
}

fn create_buf_writer(path: &Path) -> Result<BufWriter<File>> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| KeelError::IoWrite {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let file = File::create(path).map_err(|source| KeelError::IoWrite {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(BufWriter::new(file))
}
