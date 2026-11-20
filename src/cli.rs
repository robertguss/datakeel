use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "datakeel",
    version,
    about = "Blazing-fast, agent-first (DX + AX) Rust data contract, schema drift, and codegen CLI with native ODCS v3 support.",
    long_about = None
)]
pub struct Cli {
    /// Output format: auto (human in TTY, agent when piped), human, agent, or json
    #[arg(long, global = true, value_enum, default_value_t = OutputFormatArg::Auto)]
    pub format: OutputFormatArg,

    /// Disable ANSI colors in human output
    #[arg(long, global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormatArg {
    Auto,
    Human,
    Agent,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ContractFormatArg {
    /// Open Data Contract Standard v3.2.0 YAML (datacontract.yaml)
    Odcs,
    /// Compact DataKeel TOML (datakeel.toml)
    Toml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DataFormatArg {
    Auto,
    Json,
    Jsonl,
    Csv,
    Tsv,
    Parquet,
    Postgres,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CodegenTarget {
    Postgres,
    Pydantic,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Infer a data contract (ODCS v3 YAML or datakeel.toml) from JSON, JSONL, CSV, Parquet, or a live Postgres table
    Infer {
        /// Path to input dataset, '-' for stdin, 'pg://[schema.]table' (uses DATABASE_URL), or 'postgres://...'
        input: String,

        /// Explicit input data format (default: auto-detect from extension or URI scheme)
        #[arg(long = "input-format", value_enum, default_value_t = DataFormatArg::Auto)]
        input_format: DataFormatArg,

        /// Dot-separated path to an array of objects inside a JSON envelope (e.g. 'data.students')
        #[arg(long)]
        json_path: Option<String>,

        /// Postgres table name (`table` or `schema.table`) when passing a `postgres://` URI
        #[arg(long)]
        table: Option<String>,

        /// Optional SQL WHERE predicate when reading from Postgres (e.g. "status = 'active'")
        #[arg(long = "where")]
        where_clause: Option<String>,

        /// Maximum number of rows to fetch when reading from Postgres
        #[arg(long)]
        limit: Option<usize>,

        /// Contract format to emit: odcs (ODCS v3.2.0 YAML) or toml (compact datakeel.toml)
        #[arg(long = "contract-format", value_enum, default_value_t = ContractFormatArg::Odcs)]
        contract_format: ContractFormatArg,

        /// Dataset / table name to record in the contract (defaults to file stem or Postgres table)
        #[arg(long)]
        name: Option<String>,

        /// Infer enum constraints for low-cardinality string columns (<= N distinct values)
        #[arg(long, default_value_t = 0)]
        infer_enums: usize,

        /// Write generated contract to a file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Validate a dataset (file or live Postgres table) against an ODCS v3 YAML or datakeel.toml contract
    Check {
        /// Path to contract file (.yaml, .yml, or .toml)
        contract: PathBuf,

        /// Dataset path, '-' for stdin, 'pg://[schema.]table' (uses DATABASE_URL), or 'postgres://...'
        data: String,

        /// Explicit input data format (default: auto-detect from extension or URI scheme)
        #[arg(long = "input-format", value_enum, default_value_t = DataFormatArg::Auto)]
        input_format: DataFormatArg,

        /// Dot-separated path to an array of objects inside a JSON envelope
        #[arg(long)]
        json_path: Option<String>,

        /// Postgres table name (`table` or `schema.table`) when passing a `postgres://` URI
        #[arg(long)]
        table: Option<String>,

        /// Optional SQL WHERE predicate when reading from Postgres (e.g. "ingested_at >= now() - interval '1 day'")
        #[arg(long = "where")]
        where_clause: Option<String>,

        /// Maximum number of rows to fetch when reading from Postgres
        #[arg(long)]
        limit: Option<usize>,

        /// Cross-source reference datasets for foreign-key checks: NAME=PATH_OR_PG_URI (repeatable)
        #[arg(long = "ref", value_name = "NAME=SOURCE")]
        refs: Vec<String>,

        /// Fail on unexpected columns not declared in the contract (overrides contract setting)
        #[arg(long)]
        strict: bool,

        /// Maximum sample rows/values included per violation (keeps agent context compact)
        #[arg(long, default_value_t = 3)]
        max_samples: usize,

        /// Write valid rows passing all row-level contract rules to this JSONL file
        #[arg(long)]
        clean: Option<PathBuf>,

        /// Write rejected rows (annotated with exact DK_* error codes) to this JSONL file
        #[arg(long)]
        quarantine: Option<PathBuf>,

        /// Allow exit 0 when row-level quarantine rate is <= RATE (0.0 - 1.0) and no table/schema errors occur
        #[arg(long)]
        max_quarantine_rate: Option<f64>,
    },

    /// Detect schema drift, breaking type shifts, null-rate spikes, and row-count drops (files, contracts, or live Postgres tables)
    Diff {
        /// Baseline snapshot (data file, contract file, 'pg://[schema.]table', or 'postgres://...')
        baseline: String,

        /// Current snapshot (data file, contract file, 'pg://[schema.]table', or 'postgres://...')
        current: String,

        /// Explicit format for baseline if data source
        #[arg(long = "baseline-format", value_enum, default_value_t = DataFormatArg::Auto)]
        baseline_format: DataFormatArg,

        /// Explicit format for current if data source
        #[arg(long = "current-format", value_enum, default_value_t = DataFormatArg::Auto)]
        current_format: DataFormatArg,

        /// Dot-separated JSON path for JSON envelopes
        #[arg(long)]
        json_path: Option<String>,

        /// Optional SQL WHERE predicate applied to Postgres sources
        #[arg(long = "where")]
        where_clause: Option<String>,

        /// Maximum number of rows to fetch when reading from Postgres
        #[arg(long)]
        limit: Option<usize>,

        /// Maximum allowed increase in column null rate before flagging breaking drift (0.0 - 1.0)
        #[arg(long, default_value_t = 0.05)]
        max_null_delta: f64,

        /// Maximum allowed percentage drop in row count before flagging breaking drift (0.0 - 100.0)
        #[arg(long, default_value_t = 20.0)]
        max_row_drop_pct: f64,

        /// Exit with code 1 on any drift (including non-breaking additions)
        #[arg(long)]
        fail_on_any: bool,
    },

    /// Generate Postgres CREATE TABLE DDL or Python Pydantic v2 models from a contract
    Codegen {
        /// Path to contract file (.yaml, .yml, or .toml)
        contract: PathBuf,

        /// Target language/dialect
        #[arg(long, value_enum)]
        target: CodegenTarget,

        /// Postgres schema prefix (e.g. 'staging' or 'public') for postgres target
        #[arg(long)]
        schema: Option<String>,

        /// Write generated code to a file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Lint an ODCS v3 YAML or datakeel.toml contract file for syntax and semantic issues
    Lint {
        /// Path to contract file (.yaml, .yml, or .toml)
        contract: PathBuf,
    },

    /// Print or install the bundled SKILL.md agent manual (for Claude Code, Antigravity, Codex, Cursor)
    Skill {
        /// Install SKILL.md into a directory (e.g. .agents/skills/datakeel/SKILL.md)
        #[arg(long)]
        install: Option<PathBuf>,
    },
}
