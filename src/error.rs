use serde::Serialize;
use std::path::PathBuf;
use thiserror::Error;

/// Canonical exit codes for deterministic human and agent branching.
pub const EXIT_OK: i32 = 0;
pub const EXIT_VIOLATION: i32 = 1;
pub const EXIT_ERROR: i32 = 2;

#[derive(Debug, Error)]
pub enum KeelError {
    #[error("Failed to read file '{path}': {source}")]
    IoRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to write file '{path}': {source}")]
    IoWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Unsupported file format for '{path}': {detail}")]
    UnsupportedFormat { path: String, detail: String },

    #[error("Failed to parse contract '{path}': {detail}")]
    ContractParse { path: String, detail: String },

    #[error("Failed to parse data '{path}': {detail}")]
    DataParse { path: String, detail: String },

    #[error("Invalid reference argument '{raw}': expected NAME=PATH (e.g. populi_students=students.jsonl)")]
    InvalidRefArg { raw: String },

    #[error("Invalid regex pattern '{pattern}' on column '{column}': {detail}")]
    InvalidRegex {
        column: String,
        pattern: String,
        detail: String,
    },

    #[error("JSON path '{path}' not found or not an array of objects in '{source_name}'")]
    JsonPathNotFound { path: String, source_name: String },

    #[error("{0}")]
    General(String),
}

impl KeelError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::IoRead { .. } => "DK_IO_READ",
            Self::IoWrite { .. } => "DK_IO_WRITE",
            Self::UnsupportedFormat { .. } => "DK_UNSUPPORTED_FORMAT",
            Self::ContractParse { .. } => "DK_CONTRACT_PARSE",
            Self::DataParse { .. } => "DK_DATA_PARSE",
            Self::InvalidRefArg { .. } => "DK_INVALID_REF_ARG",
            Self::InvalidRegex { .. } => "DK_INVALID_REGEX",
            Self::JsonPathNotFound { .. } => "DK_JSON_PATH_NOT_FOUND",
            Self::General(_) => "DK_ERROR",
        }
    }

    pub fn fix_hint(&self) -> Option<String> {
        match self {
            Self::IoRead { path, .. } => Some(format!(
                "Verify that '{}' exists and is readable.",
                path.display()
            )),
            Self::UnsupportedFormat { .. } => Some(
                "Supported data formats: .json, .jsonl, .ndjson, .csv, .tsv, .parquet (or pass --input-format). Supported contract formats: .yaml, .yml (ODCS v3), .toml."
                    .to_string(),
            ),
            Self::ContractParse { path, .. } => Some(format!(
                "Run `datakeel lint {}` to inspect contract structure and fix syntax errors.",
                path
            )),
            Self::InvalidRefArg { .. } => Some(
                "Pass cross-source references as `--ref <table_name>=<file_path>`.".to_string(),
            ),
            Self::JsonPathNotFound { path, .. } => Some(format!(
                "Check the top-level JSON structure or omit `--json-path {}` if the root is already an array or JSONL.",
                path
            )),
            Self::InvalidRegex { column, .. } => Some(format!(
                "Fix the regular expression pattern for column '{}' in the contract.",
                column
            )),
            _ => None,
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(self, Self::IoRead { .. } | Self::IoWrite { .. })
    }

    pub fn to_envelope(&self) -> ErrorEnvelope {
        ErrorEnvelope {
            schema_version: "1.0",
            ok: false,
            error: ErrorBody {
                code: self.code(),
                message: self.to_string(),
                fix_hint: self.fix_hint(),
                retryable: self.retryable(),
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorEnvelope {
    pub schema_version: &'static str,
    pub ok: bool,
    pub error: ErrorBody,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix_hint: Option<String>,
    pub retryable: bool,
}

pub type Result<T> = std::result::Result<T, KeelError>;
