use crate::data::{Dataset, Row, Value};
use crate::error::{KeelError, Result};
use indexmap::IndexMap;
use serde_json::Value as JsonValue;

pub fn read_json(bytes: &[u8], source_name: &str, json_path: Option<&str>) -> Result<Dataset> {
    let parsed: JsonValue =
        serde_json::from_slice(bytes).map_err(|e| KeelError::DataParse {
            path: source_name.to_string(),
            detail: e.to_string(),
        })?;

    let target = if let Some(path) = json_path {
        let mut cursor = &parsed;
        for segment in path.split('.') {
            cursor = cursor
                .get(segment)
                .ok_or_else(|| KeelError::JsonPathNotFound {
                    path: path.to_string(),
                    source_name: source_name.to_string(),
                })?;
        }
        cursor
    } else {
        &parsed
    };

    let arr = target.as_array().ok_or_else(|| {
        if let Some(path) = json_path {
            KeelError::JsonPathNotFound {
                path: path.to_string(),
                source_name: source_name.to_string(),
            }
        } else {
            KeelError::DataParse {
                path: source_name.to_string(),
                detail: "expected a top-level JSON array of objects (or pass --json-path to unwrap an envelope)".to_string(),
            }
        }
    })?;

    let mut rows = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        rows.push(json_object_to_row(item, source_name, idx + 1)?);
    }

    Ok(Dataset::from_rows(source_name.to_string(), rows))
}

pub fn read_jsonl(bytes: &[u8], source_name: &str) -> Result<Dataset> {
    let text = std::str::from_utf8(bytes).map_err(|e| KeelError::DataParse {
        path: source_name.to_string(),
        detail: format!("invalid UTF-8: {e}"),
    })?;

    let mut rows = Vec::new();
    for (line_idx, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed: JsonValue =
            serde_json::from_str(line).map_err(|e| KeelError::DataParse {
                path: source_name.to_string(),
                detail: format!("line {}: {e}", line_idx + 1),
            })?;
        rows.push(json_object_to_row(&parsed, source_name, line_idx + 1)?);
    }

    Ok(Dataset::from_rows(source_name.to_string(), rows))
}

fn json_object_to_row(val: &JsonValue, source_name: &str, row_num: usize) -> Result<Row> {
    let map = val.as_object().ok_or_else(|| KeelError::DataParse {
        path: source_name.to_string(),
        detail: format!("record {row_num} must be a JSON object"),
    })?;
    let mut row = IndexMap::with_capacity(map.len());
    for (k, v) in map {
        row.insert(k.clone(), json_to_value(v));
    }
    Ok(row)
}

fn json_to_value(v: &JsonValue) -> Value {
    match v {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(b) => Value::Bool(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(u) = n.as_u64() {
                if u <= i64::MAX as u64 {
                    Value::Int(u as i64)
                } else {
                    Value::Float(u as f64)
                }
            } else {
                Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        JsonValue::String(s) => Value::classify_string(s.clone()),
        JsonValue::Array(arr) => Value::Array(arr.iter().map(json_to_value).collect()),
        JsonValue::Object(obj) => {
            let mut m = IndexMap::with_capacity(obj.len());
            for (k, v) in obj {
                m.insert(k.clone(), json_to_value(v));
            }
            Value::Object(m)
        }
    }
}
