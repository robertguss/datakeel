pub mod odcs;
pub mod toml_fmt;

use crate::error::{KeelError, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractFormat {
    OdcsYaml,
    Toml,
}

/// Canonical logical data types aligned with ODCS v3 and SQL/Python type systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogicalType {
    String,
    Integer,
    Number,
    Boolean,
    Date,
    Timestamp,
    Array,
    Object,
    Any,
}

impl LogicalType {
    pub fn default_postgres_type(self) -> &'static str {
        match self {
            Self::String => "text",
            Self::Integer => "bigint",
            Self::Number => "double precision",
            Self::Boolean => "boolean",
            Self::Date => "date",
            Self::Timestamp => "timestamptz",
            Self::Array => "jsonb",
            Self::Object => "jsonb",
            Self::Any => "jsonb",
        }
    }

    pub fn python_type(self) -> &'static str {
        match self {
            Self::String => "str",
            Self::Integer => "int",
            Self::Number => "float",
            Self::Boolean => "bool",
            Self::Date => "date",
            Self::Timestamp => "datetime",
            Self::Array => "list[Any]",
            Self::Object => "dict[str, Any]",
            Self::Any => "Any",
        }
    }

    /// Type-widening compatibility lattice: can a value of `actual` safely fit in `expected`?
    pub fn is_compatible_with(actual: Self, expected: Self) -> bool {
        if actual == expected || expected == Self::Any {
            return true;
        }
        matches!(
            (actual, expected),
            (Self::Integer, Self::Number)
                | (Self::Date, Self::Timestamp)
                | (Self::Date, Self::String)
                | (Self::Timestamp, Self::String)
        )
    }

    /// True if evolving a baseline column from `self` -> `next` is a safe widening.
    pub fn is_safe_widening_to(self, next: Self) -> bool {
        if self == next || next == Self::Any {
            return true;
        }
        matches!(
            (self, next),
            (Self::Integer, Self::Number) | (Self::Date, Self::Timestamp)
        )
    }

    pub fn from_odcs_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "string" | "text" | "varchar" | "char" | "uuid" => Self::String,
            "integer" | "int" | "bigint" | "smallint" | "int64" | "int32" => Self::Integer,
            "number" | "float" | "double" | "decimal" | "numeric" | "real" => Self::Number,
            "boolean" | "bool" => Self::Boolean,
            "date" => Self::Date,
            "timestamp" | "datetime" | "timestamptz" => Self::Timestamp,
            "array" | "list" => Self::Array,
            "object" | "struct" | "map" | "json" | "jsonb" => Self::Object,
            _ => Self::Any,
        }
    }
}

impl fmt::Display for LogicalType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Date => "date",
            Self::Timestamp => "timestamp",
            Self::Array => "array",
            Self::Object => "object",
            Self::Any => "any",
        };
        f.write_str(s)
    }
}

/// Canonical internal representation of a DataKeel data contract.
/// Both ODCS v3 YAML (`datacontract.yaml`) and compact `datakeel.toml` compile into this model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    pub api_version: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Physical table / object name (defaults to `name`).
    pub table: String,
    /// When true, unexpected columns not listed in `columns` fail validation.
    #[serde(default)]
    pub strict: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_rows: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rows: Option<usize>,
    pub columns: IndexMap<String, ColumnContract>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnContract {
    pub logical_type: LogicalType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_type: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub primary_key: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Maximum allowed fraction of null values (0.0 ..= 1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_null_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<String>>,
    /// Cross-source foreign key reference in `dataset.column` syntax (e.g. `populi_students.student_id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub references: Option<String>,
}

impl Contract {
    pub fn detect_format(path: &Path) -> Result<ContractFormat> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "yaml" | "yml" => Ok(ContractFormat::OdcsYaml),
            "toml" => Ok(ContractFormat::Toml),
            _ => Err(KeelError::UnsupportedFormat {
                path: path.display().to_string(),
                detail: format!("unrecognized contract extension '.{ext}'"),
            }),
        }
    }

    pub fn is_contract_path(path: &Path) -> bool {
        Self::detect_format(path).is_ok()
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        let fmt = Self::detect_format(path)?;
        let content = fs::read_to_string(path).map_err(|source| KeelError::IoRead {
            path: path.to_path_buf(),
            source,
        })?;
        match fmt {
            ContractFormat::OdcsYaml => odcs::parse_odcs(&content, &path.display().to_string()),
            ContractFormat::Toml => toml_fmt::parse_toml(&content, &path.display().to_string()),
        }
    }

    pub fn serialize(&self, fmt: ContractFormat) -> Result<String> {
        match fmt {
            ContractFormat::OdcsYaml => odcs::serialize_odcs(self),
            ContractFormat::Toml => toml_fmt::serialize_toml(self),
        }
    }
}
