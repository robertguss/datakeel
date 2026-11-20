pub mod csv_reader;
pub mod json;
pub mod parquet_reader;

use crate::cli::DataFormatArg;
use crate::contract::LogicalType;
use crate::error::{KeelError, Result};
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use indexmap::{IndexMap, IndexSet};
use serde::Serialize;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Date(String),
    Timestamp(String),
    String(String),
    Array(Vec<Value>),
    Object(IndexMap<String, Value>),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    pub fn logical_type(&self) -> Option<LogicalType> {
        match self {
            Self::Null => None,
            Self::Bool(_) => Some(LogicalType::Boolean),
            Self::Int(_) => Some(LogicalType::Integer),
            Self::Float(_) => Some(LogicalType::Number),
            Self::Date(_) => Some(LogicalType::Date),
            Self::Timestamp(_) => Some(LogicalType::Timestamp),
            Self::String(_) => Some(LogicalType::String),
            Self::Array(_) => Some(LogicalType::Array),
            Self::Object(_) => Some(LogicalType::Object),
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(i) => Some(*i as f64),
            Self::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) | Self::Date(s) | Self::Timestamp(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Canonical string key for uniqueness & cross-source foreign-key lookups.
    pub fn canonical_key(&self) -> Option<String> {
        match self {
            Self::Null => None,
            Self::Bool(b) => Some(b.to_string()),
            Self::Int(i) => Some(i.to_string()),
            Self::Float(f) => Some(format!("{f}")),
            Self::Date(s) | Self::Timestamp(s) | Self::String(s) => Some(s.clone()),
            Self::Array(_) | Self::Object(_) => serde_json::to_string(self).ok(),
        }
    }

    pub fn classify_string(s: String) -> Self {
        let trimmed = s.trim();
        if is_iso_date(trimmed) {
            return Self::Date(trimmed.to_string());
        }
        if is_iso_timestamp(trimmed) {
            return Self::Timestamp(trimmed.to_string());
        }
        Self::String(s)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("null"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int(i) => write!(f, "{i}"),
            Self::Float(v) => write!(f, "{v}"),
            Self::Date(s) | Self::Timestamp(s) | Self::String(s) => write!(f, "{s}"),
            Self::Array(_) | Self::Object(_) => {
                let s = serde_json::to_string(self).unwrap_or_else(|_| "<complex>".to_string());
                f.write_str(&s)
            }
        }
    }
}

pub fn is_iso_date(s: &str) -> bool {
    s.len() == 10 && NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

pub fn is_iso_timestamp(s: &str) -> bool {
    if s.len() < 19 || !s.as_bytes().get(4).is_some_and(|&b| b == b'-') {
        return false;
    }
    DateTime::parse_from_rfc3339(s).is_ok()
        || NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").is_ok()
        || NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").is_ok()
        || NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f").is_ok()
        || NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").is_ok()
}

pub type Row = IndexMap<String, Value>;

#[derive(Debug, Clone)]
pub struct Dataset {
    pub source_name: String,
    pub columns: IndexSet<String>,
    pub rows: Vec<Row>,
}

impl Dataset {
    pub fn from_rows(source_name: String, rows: Vec<Row>) -> Self {
        let mut columns = IndexSet::new();
        for row in &rows {
            for k in row.keys() {
                columns.insert(k.clone());
            }
        }
        Self {
            source_name,
            columns,
            rows,
        }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

pub fn detect_data_format(input: &str, explicit: DataFormatArg) -> Result<DataFormatArg> {
    if explicit != DataFormatArg::Auto {
        return Ok(explicit);
    }
    if input == "-" {
        return Ok(DataFormatArg::Jsonl);
    }
    let ext = Path::new(input)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "json" => Ok(DataFormatArg::Json),
        "jsonl" | "ndjson" => Ok(DataFormatArg::Jsonl),
        "csv" => Ok(DataFormatArg::Csv),
        "tsv" => Ok(DataFormatArg::Tsv),
        "parquet" | "pq" => Ok(DataFormatArg::Parquet),
        _ => Err(KeelError::UnsupportedFormat {
            path: input.to_string(),
            detail: format!("could not detect data format from extension '.{ext}'"),
        }),
    }
}

pub fn load_dataset(
    input: &str,
    explicit_format: DataFormatArg,
    json_path: Option<&str>,
) -> Result<Dataset> {
    let fmt = detect_data_format(input, explicit_format)?;

    if fmt == DataFormatArg::Parquet {
        if input == "-" {
            return Err(KeelError::UnsupportedFormat {
                path: "-".to_string(),
                detail: "Parquet requires a seekable file path, not stdin".to_string(),
            });
        }
        return parquet_reader::read_parquet(Path::new(input));
    }

    let raw_bytes = if input == "-" {
        let mut buf = Vec::new();
        io::stdin()
            .read_to_end(&mut buf)
            .map_err(|source| KeelError::IoRead {
                path: Path::new("-").to_path_buf(),
                source,
            })?;
        buf
    } else {
        fs::read(input).map_err(|source| KeelError::IoRead {
            path: Path::new(input).to_path_buf(),
            source,
        })?
    };

    match fmt {
        DataFormatArg::Json => json::read_json(&raw_bytes, input, json_path),
        DataFormatArg::Jsonl => json::read_jsonl(&raw_bytes, input),
        DataFormatArg::Csv => csv_reader::read_csv(&raw_bytes, input, b','),
        DataFormatArg::Tsv => csv_reader::read_csv(&raw_bytes, input, b'\t'),
        DataFormatArg::Parquet | DataFormatArg::Auto => unreachable!(),
    }
}
