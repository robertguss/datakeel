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

    #[error("Postgres shorthand '{target}' requires DATABASE_URL or DATAKEEL_DATABASE_URL to be set")]
    PgMissingDatabaseUrl { target: String },

    #[error("Postgres URI '{target}' is missing a table name (pass `--table schema.table` or `?table=schema.table`)")]
    PgMissingTable { target: String },

    #[error("Postgres table '{table}' does not exist or is not accessible in '{target}'")]
    PgTableNotFound { target: String, table: String },

    #[error("Failed to connect to Postgres '{target}': {detail}")]
    PgConnect { target: String, detail: String },

    #[error("Postgres query failed on '{target}': {detail}")]
    PgQuery { target: String, detail: String },

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
            Self::PgMissingDatabaseUrl { .. } => "DK_PG_MISSING_DATABASE_URL",
            Self::PgMissingTable { .. } => "DK_PG_MISSING_TABLE",
            Self::PgTableNotFound { .. } => "DK_PG_TABLE_NOT_FOUND",
            Self::PgConnect { .. } => "DK_PG_CONNECT",
            Self::PgQuery { .. } => "DK_PG_QUERY",
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
                "Supported data formats: .json, .jsonl, .ndjson, .csv, .tsv, .parquet, pg://schema.table, postgres://... (or pass --input-format). Supported contract formats: .yaml, .yml (ODCS v3), .toml."
                    .to_string(),
            ),
            Self::ContractParse { path, .. } => Some(format!(
                "Run `datakeel lint {}` to inspect contract structure and fix syntax errors.",
                path
            )),
            Self::InvalidRefArg { .. } => Some(
                "Pass cross-source references as `--ref <table_name>=<file_or_pg_uri>` (e.g. `--ref populi_students=pg://staging.populi_students`).".to_string(),
            ),
            Self::JsonPathNotFound { path, .. } => Some(format!(
                "Check the top-level JSON structure or omit `--json-path {}` if the root is already an array or JSONL.",
                path
            )),
            Self::InvalidRegex { column, .. } => Some(format!(
                "Fix the regular expression pattern for column '{}' in the contract.",
                column
            )),
            Self::PgMissingDatabaseUrl { .. } => Some(
                "Set `export DATABASE_URL=postgres://user:pass@host:5432/dbname` (or `DATAKEEL_DATABASE_URL`) and retry."
                    .to_string(),
            ),
            Self::PgMissingTable { .. } => Some(
                "Specify the target table via `--table staging.my_table` or append `?table=staging.my_table` to the Postgres URI (or use `pg://staging.my_table` with `DATABASE_URL`)."
                    .to_string(),
            ),
            Self::PgTableNotFound { table, .. } => Some(format!(
                "Verify that table '{table}' exists and the Postgres role has SELECT permission (include schema prefix if not in 'public', e.g. `pg://staging.populi_students`)."
            )),
            Self::PgConnect { .. } => Some(
                "Check host, port, credentials, network reachability, and `sslmode` (e.g. `?sslmode=disable` for local Postgres or `?sslmode=require` for hosted Postgres)."
                    .to_string(),
            ),
            Self::PgQuery { .. } => Some(
                "Verify `--where` filter syntax and column names against the Postgres table."
                    .to_string(),
            ),
            _ => None,
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::IoRead { .. } | Self::IoWrite { .. } | Self::PgConnect { .. }
        )
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
