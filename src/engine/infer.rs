use crate::contract::odcs::ODCS_API_VERSION;
use crate::contract::{ColumnContract, Contract, LogicalType};
use crate::data::{Dataset, Value};
use indexmap::IndexMap;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy)]
pub struct InferOptions {
    pub max_enum_cardinality: usize,
}

#[derive(Debug, Clone)]
pub struct ColumnStats {
    pub inferred_type: LogicalType,
    pub total_rows: usize,
    pub null_count: usize,
    pub null_rate: f64,
    pub distinct_non_null: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub distinct_strings: Option<Vec<String>>,
}

pub fn profile_dataset(dataset: &Dataset, max_enum_cardinality: usize) -> IndexMap<String, ColumnStats> {
    let total_rows = dataset.row_count();
    let mut stats = IndexMap::new();

    for col_name in &dataset.columns {
        let mut unified_type: Option<LogicalType> = None;
        let mut null_count = 0usize;
        let mut min_val: Option<f64> = None;
        let mut max_val: Option<f64> = None;
        let mut seen_keys: HashSet<String> = HashSet::new();
        let mut string_set: HashSet<String> = HashSet::new();
        let mut track_strings = max_enum_cardinality > 0;

        for row in &dataset.rows {
            let val = row.get(col_name).unwrap_or(&Value::Null);
            if val.is_null() {
                null_count += 1;
                continue;
            }

            if let Some(t) = val.logical_type() {
                unified_type = Some(match unified_type {
                    None => t,
                    Some(prev) => unify_types(prev, t),
                });
            }

            if let Some(num) = val.as_f64() {
                min_val = Some(match min_val {
                    Some(m) => m.min(num),
                    None => num,
                });
                max_val = Some(match max_val {
                    Some(m) => m.max(num),
                    None => num,
                });
            }

            if let Some(key) = val.canonical_key() {
                seen_keys.insert(key);
            }

            if track_strings {
                if let Value::String(s) = val {
                    string_set.insert(s.clone());
                    if string_set.len() > max_enum_cardinality {
                        track_strings = false;
                        string_set.clear();
                    }
                } else {
                    track_strings = false;
                    string_set.clear();
                }
            }
        }

        let inferred_type = unified_type.unwrap_or(LogicalType::String);
        let null_rate = if total_rows == 0 {
            0.0
        } else {
            null_count as f64 / total_rows as f64
        };

        let distinct_strings = if track_strings
            && inferred_type == LogicalType::String
            && !string_set.is_empty()
            && string_set.len() <= max_enum_cardinality
        {
            let mut v: Vec<String> = string_set.into_iter().collect();
            v.sort();
            Some(v)
        } else {
            None
        };

        let (min, max) = if matches!(inferred_type, LogicalType::Integer | LogicalType::Number) {
            (min_val, max_val)
        } else {
            (None, None)
        };

        stats.insert(
            col_name.clone(),
            ColumnStats {
                inferred_type,
                total_rows,
                null_count,
                null_rate,
                distinct_non_null: seen_keys.len(),
                min,
                max,
                distinct_strings,
            },
        );
    }

    stats
}

pub fn infer_contract(dataset: &Dataset, name: &str, options: InferOptions) -> Contract {
    let stats = profile_dataset(dataset, options.max_enum_cardinality);
    let total_rows = dataset.row_count();
    let mut columns = IndexMap::new();
    let mut assigned_pk = false;

    for (col_name, st) in stats {
        let required = total_rows > 0 && st.null_count == 0;
        let unique = required && total_rows >= 2 && st.distinct_non_null == total_rows;
        let looks_like_id = col_name == "id" || col_name.ends_with("_id");
        let primary_key = if !assigned_pk && unique && looks_like_id {
            assigned_pk = true;
            true
        } else {
            false
        };

        columns.insert(
            col_name,
            ColumnContract {
                logical_type: st.inferred_type,
                physical_type: Some(st.inferred_type.default_postgres_type().to_string()),
                required,
                unique,
                primary_key,
                description: None,
                max_null_rate: if !required && st.null_rate > 0.0 {
                    // Round observed null rate up slightly as a sensible starter budget.
                    Some(((st.null_rate * 100.0).ceil() / 100.0).min(1.0))
                } else {
                    None
                },
                min: st.min,
                max: st.max,
                min_length: None,
                max_length: None,
                pattern: None,
                allowed_values: st.distinct_strings,
                references: None,
            },
        );
    }

    Contract {
        api_version: ODCS_API_VERSION.to_string(),
        id: format!("urn:datakeel:{name}"),
        name: name.to_string(),
        version: "1.0.0".to_string(),
        status: "active".to_string(),
        description: Some(format!(
            "Inferred by datakeel from {} ({} rows).",
            dataset.source_name, total_rows
        )),
        table: name.to_string(),
        strict: false,
        min_rows: if total_rows > 0 { Some(1) } else { None },
        max_rows: None,
        columns,
    }
}

pub fn unify_types(a: LogicalType, b: LogicalType) -> LogicalType {
    if a == b {
        return a;
    }
    match (a, b) {
        (LogicalType::Integer, LogicalType::Number)
        | (LogicalType::Number, LogicalType::Integer) => LogicalType::Number,
        (LogicalType::Date, LogicalType::Timestamp)
        | (LogicalType::Timestamp, LogicalType::Date) => LogicalType::Timestamp,
        (LogicalType::Date, LogicalType::String)
        | (LogicalType::String, LogicalType::Date)
        | (LogicalType::Timestamp, LogicalType::String)
        | (LogicalType::String, LogicalType::Timestamp) => LogicalType::String,
        _ => LogicalType::Any,
    }
}
