use crate::data::{Dataset, Row, Value};
use crate::error::{KeelError, Result};
use chrono::{DateTime, Duration, NaiveDate};
use indexmap::{IndexMap, IndexSet};
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::Field;
use std::fs::File;
use std::path::Path;

pub fn read_parquet(path: &Path) -> Result<Dataset> {
    let file = File::open(path).map_err(|source| KeelError::IoRead {
        path: path.to_path_buf(),
        source,
    })?;
    let reader = SerializedFileReader::new(file).map_err(|e| KeelError::DataParse {
        path: path.display().to_string(),
        detail: format!("failed to open Parquet file: {e}"),
    })?;

    let schema = reader.metadata().file_metadata().schema();
    let mut columns = IndexSet::new();
    for field in schema.get_fields() {
        columns.insert(field.name().to_string());
    }

    let iter = reader.get_row_iter(None).map_err(|e| KeelError::DataParse {
        path: path.display().to_string(),
        detail: format!("failed to iterate Parquet rows: {e}"),
    })?;

    let mut rows = Vec::new();
    for row_res in iter {
        let pq_row = row_res.map_err(|e| KeelError::DataParse {
            path: path.display().to_string(),
            detail: format!("failed to read Parquet row: {e}"),
        })?;
        let mut row: Row = IndexMap::with_capacity(pq_row.len());
        for (name, field) in pq_row.get_column_iter() {
            row.insert(name.clone(), field_to_value(field));
        }
        rows.push(row);
    }

    Ok(Dataset {
        source_name: path.display().to_string(),
        columns,
        rows,
    })
}

fn field_to_value(field: &Field) -> Value {
    match field {
        Field::Null => Value::Null,
        Field::Bool(b) => Value::Bool(*b),
        Field::Byte(v) => Value::Int(*v as i64),
        Field::Short(v) => Value::Int(*v as i64),
        Field::Int(v) => Value::Int(*v as i64),
        Field::Long(v) => Value::Int(*v),
        Field::UByte(v) => Value::Int(*v as i64),
        Field::UShort(v) => Value::Int(*v as i64),
        Field::UInt(v) => Value::Int(*v as i64),
        Field::ULong(v) => {
            if *v <= i64::MAX as u64 {
                Value::Int(*v as i64)
            } else {
                Value::Float(*v as f64)
            }
        }
        Field::Float(v) => Value::Float(*v as f64),
        Field::Double(v) => Value::Float(*v),
        Field::Str(s) => Value::classify_string(s.clone()),
        Field::Date(days) => {
            let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid epoch");
            let date = epoch + Duration::days(*days as i64);
            Value::Date(date.format("%Y-%m-%d").to_string())
        }
        Field::TimestampMillis(ms) => {
            if let Some(dt) = DateTime::from_timestamp_millis(*ms) {
                Value::Timestamp(dt.to_rfc3339())
            } else {
                Value::Int(*ms)
            }
        }
        Field::TimestampMicros(us) => {
            if let Some(dt) = DateTime::from_timestamp_micros(*us) {
                Value::Timestamp(dt.to_rfc3339())
            } else {
                Value::Int(*us)
            }
        }
        Field::Group(sub) => {
            let mut m = IndexMap::with_capacity(sub.len());
            for (k, v) in sub.get_column_iter() {
                m.insert(k.clone(), field_to_value(v));
            }
            Value::Object(m)
        }
        Field::ListInternal(list) => {
            Value::Array(list.elements().iter().map(field_to_value).collect())
        }
        Field::MapInternal(map) => {
            let mut m = IndexMap::new();
            for (k, v) in map.entries() {
                let key = field_to_value(k).to_string();
                m.insert(key, field_to_value(v));
            }
            Value::Object(m)
        }
        other => Value::String(other.to_string()),
    }
}
