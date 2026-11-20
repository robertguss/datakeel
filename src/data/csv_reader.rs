use crate::data::{Dataset, Row, Value};
use crate::error::{KeelError, Result};
use indexmap::{IndexMap, IndexSet};

pub fn read_csv(bytes: &[u8], source_name: &str, delimiter: u8) -> Result<Dataset> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(true)
        .flexible(true)
        .from_reader(bytes);

    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| KeelError::DataParse {
            path: source_name.to_string(),
            detail: format!("failed to parse CSV headers: {e}"),
        })?
        .iter()
        .map(|h| h.trim().to_string())
        .collect();

    let mut columns = IndexSet::new();
    for h in &headers {
        columns.insert(h.clone());
    }

    let mut rows = Vec::new();
    for (idx, record_res) in reader.records().enumerate() {
        let record = record_res.map_err(|e| KeelError::DataParse {
            path: source_name.to_string(),
            detail: format!("row {}: {e}", idx + 2),
        })?;
        let mut row: Row = IndexMap::with_capacity(headers.len());
        for (col_idx, col_name) in headers.iter().enumerate() {
            let raw = record.get(col_idx).unwrap_or("");
            row.insert(col_name.clone(), parse_csv_cell(raw));
        }
        rows.push(row);
    }

    Ok(Dataset {
        source_name: source_name.to_string(),
        columns,
        rows,
    })
}

fn parse_csv_cell(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::Null;
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    // Avoid turning leading-zero IDs like "00123" into integers!
    let has_leading_zero = trimmed.len() > 1
        && trimmed.starts_with('0')
        && !trimmed.starts_with("0.");
    if !has_leading_zero {
        if let Ok(i) = trimmed.parse::<i64>() {
            return Value::Int(i);
        }
        if let Ok(f) = trimmed.parse::<f64>()
            && f.is_finite()
        {
            return Value::Float(f);
        }
    }
    Value::classify_string(trimmed.to_string())
}
