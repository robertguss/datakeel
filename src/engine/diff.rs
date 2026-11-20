use crate::cli::DataFormatArg;
use crate::contract::{Contract, LogicalType};
use crate::data::load_dataset;
use crate::engine::infer::profile_dataset;
use crate::error::Result;
use indexmap::IndexMap;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct DiffOptions {
    pub baseline_format: DataFormatArg,
    pub current_format: DataFormatArg,
    pub json_path: Option<String>,
    pub max_null_delta: f64,
    pub max_row_drop_pct: f64,
    pub fail_on_any: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DriftSeverity {
    Breaking,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct DriftChange {
    pub code: &'static str,
    pub severity: DriftSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    pub message: String,
    pub fix_hint: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffReport {
    pub schema_version: &'static str,
    pub command: &'static str,
    pub failed: bool,
    pub baseline_path: String,
    pub current_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_rows: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_rows: Option<usize>,
    pub breaking_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    pub changes: Vec<DriftChange>,
}

struct SnapshotProfile {
    row_count: Option<usize>,
    columns: IndexMap<String, ColumnSnapshot>,
}

struct ColumnSnapshot {
    logical_type: LogicalType,
    required: bool,
    null_rate: Option<f64>,
}

fn load_snapshot(
    path: &Path,
    explicit_fmt: DataFormatArg,
    json_path: Option<&str>,
) -> Result<SnapshotProfile> {
    if explicit_fmt == DataFormatArg::Auto && Contract::is_contract_path(path) {
        let contract = Contract::load_from_path(path)?;
        let mut columns = IndexMap::new();
        for (name, col) in contract.columns {
            columns.insert(
                name,
                ColumnSnapshot {
                    logical_type: col.logical_type,
                    required: col.required,
                    null_rate: if col.required {
                        Some(0.0)
                    } else {
                        col.max_null_rate
                    },
                },
            );
        }
        Ok(SnapshotProfile {
            row_count: None,
            columns,
        })
    } else {
        let ds = load_dataset(&path.display().to_string(), explicit_fmt, json_path)?;
        let stats = profile_dataset(&ds, 0);
        let mut columns = IndexMap::new();
        for (name, st) in stats {
            columns.insert(
                name,
                ColumnSnapshot {
                    logical_type: st.inferred_type,
                    required: st.total_rows > 0 && st.null_count == 0,
                    null_rate: Some(st.null_rate),
                },
            );
        }
        Ok(SnapshotProfile {
            row_count: Some(ds.row_count()),
            columns,
        })
    }
}

pub fn diff_sources(baseline: &Path, current: &Path, options: DiffOptions) -> Result<DiffReport> {
    let base = load_snapshot(baseline, options.baseline_format, options.json_path.as_deref())?;
    let curr = load_snapshot(current, options.current_format, options.json_path.as_deref())?;

    let mut changes = Vec::new();

    // 1. Row-count drop check (catches truncated API pagination / failed syncs)
    if let (Some(b_rows), Some(c_rows)) = (base.row_count, curr.row_count)
        && b_rows > 0
        && c_rows < b_rows
    {
        let drop_pct = ((b_rows - c_rows) as f64 / b_rows as f64) * 100.0;
        if drop_pct > options.max_row_drop_pct {
            changes.push(DriftChange {
                code: "DK_DRIFT_ROW_DROP",
                severity: DriftSeverity::Breaking,
                column: None,
                message: format!(
                    "Row count dropped by {:.1}% ({b_rows} -> {c_rows}), exceeding threshold of {:.1}%.",
                    drop_pct, options.max_row_drop_pct
                ),
                fix_hint: "Inspect upstream API pagination, rate limiting, or query filters before overwriting Postgres tables.".to_string(),
            });
        }
    }

    // 2. Compare baseline columns against current
    for (col_name, b_col) in &base.columns {
        match curr.columns.get(col_name) {
            None => {
                changes.push(DriftChange {
                    code: "DK_DRIFT_COLUMN_DROPPED",
                    severity: DriftSeverity::Breaking,
                    column: Some(col_name.clone()),
                    message: format!(
                        "Column '{col_name}' ({}) present in baseline was dropped in current snapshot.",
                        b_col.logical_type
                    ),
                    fix_hint: format!(
                        "Check if the upstream API renamed or removed '{col_name}'. Update ETL mappings and downstream Postgres views before loading."
                    ),
                });
            }
            Some(c_col) => {
                if b_col.logical_type != c_col.logical_type {
                    if b_col.logical_type.is_safe_widening_to(c_col.logical_type) {
                        changes.push(DriftChange {
                            code: "DK_DRIFT_TYPE_WIDENED",
                            severity: DriftSeverity::Info,
                            column: Some(col_name.clone()),
                            message: format!(
                                "Column '{col_name}' safely widened from '{}' to '{}'.",
                                b_col.logical_type, c_col.logical_type
                            ),
                            fix_hint: format!(
                                "Verify downstream Postgres column type accommodates '{}'.",
                                c_col.logical_type
                            ),
                        });
                    } else {
                        changes.push(DriftChange {
                            code: "DK_DRIFT_TYPE_INCOMPATIBLE",
                            severity: DriftSeverity::Breaking,
                            column: Some(col_name.clone()),
                            message: format!(
                                "Column '{col_name}' changed incompatibly from '{}' to '{}'.",
                                b_col.logical_type, c_col.logical_type
                            ),
                            fix_hint: format!(
                                "Inspect raw payload values for '{col_name}' and add explicit casting/quarantine before Postgres load."
                            ),
                        });
                    }
                }

                // Null-rate spike check
                if let (Some(b_null), Some(c_null)) = (b_col.null_rate, c_col.null_rate) {
                    let delta = c_null - b_null;
                    if delta > options.max_null_delta {
                        changes.push(DriftChange {
                            code: "DK_DRIFT_NULL_SPIKE",
                            severity: DriftSeverity::Breaking,
                            column: Some(col_name.clone()),
                            message: format!(
                                "Column '{col_name}' null rate spiked from {:.1}% to {:.1}% (+{:.1}pp > {:.1}pp threshold).",
                                b_null * 100.0,
                                c_null * 100.0,
                                delta * 100.0,
                                options.max_null_delta * 100.0
                            ),
                            fix_hint: format!(
                                "Verify upstream API field population for '{col_name}'; sudden nulls can break risk-scoring features."
                            ),
                        });
                    }
                } else if b_col.required && !c_col.required {
                    changes.push(DriftChange {
                        code: "DK_DRIFT_NULLABILITY_RELAXED",
                        severity: DriftSeverity::Warning,
                        column: Some(col_name.clone()),
                        message: format!(
                            "Column '{col_name}' changed from required (non-null) to nullable."
                        ),
                        fix_hint: format!(
                            "Ensure downstream SQL queries handle NULLs on '{col_name}' safely (e.g. COALESCE / avoid NOT IN)."
                        ),
                    });
                }
            }
        }
    }

    // 3. New columns in current
    for (col_name, c_col) in &curr.columns {
        if !base.columns.contains_key(col_name) {
            changes.push(DriftChange {
                code: "DK_DRIFT_COLUMN_ADDED",
                severity: DriftSeverity::Info,
                column: Some(col_name.clone()),
                message: format!(
                    "New column '{col_name}' ({}) appeared in current snapshot.",
                    c_col.logical_type
                ),
                fix_hint: format!(
                    "If you want to capture '{col_name}', add it to the contract and regenerate Postgres DDL / Pydantic models with `datakeel codegen`."
                ),
            });
        }
    }

    let breaking_count = changes
        .iter()
        .filter(|c| c.severity == DriftSeverity::Breaking)
        .count();
    let warning_count = changes
        .iter()
        .filter(|c| c.severity == DriftSeverity::Warning)
        .count();
    let info_count = changes
        .iter()
        .filter(|c| c.severity == DriftSeverity::Info)
        .count();

    let failed = if options.fail_on_any {
        !changes.is_empty()
    } else {
        breaking_count > 0
    };

    Ok(DiffReport {
        schema_version: "1.0",
        command: "diff",
        failed,
        baseline_path: baseline.display().to_string(),
        current_path: current.display().to_string(),
        baseline_rows: base.row_count,
        current_rows: curr.row_count,
        breaking_count,
        warning_count,
        info_count,
        changes,
    })
}
