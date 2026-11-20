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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CodegenTarget {
    Postgres,
    Pydantic,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Infer a data contract (ODCS v3 YAML or datakeel.toml) from a JSON, JSONL, CSV, or Parquet dataset
    Infer {
        /// Path to input dataset (or '-' for stdin)
        input: String,

        /// Explicit input data format (default: auto-detect from extension)
        #[arg(long = "input-format", value_enum, default_value_t = DataFormatArg::Auto)]
        input_format: DataFormatArg,

        /// Dot-separated path to an array of objects inside a JSON envelope (e.g. 'data.students')
        #[arg(long)]
        json_path: Option<String>,

        /// Contract format to emit: odcs (ODCS v3.2.0 YAML) or toml (compact datakeel.toml)
        #[arg(long = "contract-format", value_enum, default_value_t = ContractFormatArg::Odcs)]
        contract_format: ContractFormatArg,

        /// Dataset / table name to record in the contract (defaults to file stem)
        #[arg(long)]
        name: Option<String>,

        /// Infer enum constraints for low-cardinality string columns (<= N distinct values)
        #[arg(long, default_value_t = 0)]
        infer_enums: usize,

        /// Write generated contract to a file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Validate a dataset against an ODCS v3 YAML or datakeel.toml contract
    Check {
        /// Path to contract file (.yaml, .yml, or .toml)
        contract: PathBuf,

        /// Path to dataset to validate (or '-' for stdin)
        data: String,

        /// Explicit input data format (default: auto-detect from extension)
        #[arg(long = "input-format", value_enum, default_value_t = DataFormatArg::Auto)]
        input_format: DataFormatArg,

        /// Dot-separated path to an array of objects inside a JSON envelope
        #[arg(long)]
        json_path: Option<String>,

        /// Cross-source reference datasets for foreign-key checks: NAME=PATH (repeatable)
        #[arg(long = "ref", value_name = "NAME=PATH")]
        refs: Vec<String>,

        /// Fail on unexpected columns not declared in the contract (overrides contract setting)
        #[arg(long)]
        strict: bool,

        /// Maximum sample rows/values included per violation (keeps agent context compact)
        #[arg(long, default_value_t = 3)]
        max_samples: usize,
    },

    /// Detect schema drift, breaking type shifts, null-rate spikes, and row-count drops
    Diff {
        /// Baseline snapshot (data file or contract file)
        baseline: PathBuf,

        /// Current snapshot (data file or contract file)
        current: PathBuf,

        /// Explicit format for baseline if data file
        #[arg(long = "baseline-format", value_enum, default_value_t = DataFormatArg::Auto)]
        baseline_format: DataFormatArg,

        /// Explicit format for current if data file
        #[arg(long = "current-format", value_enum, default_value_t = DataFormatArg::Auto)]
        current_format: DataFormatArg,

        /// Dot-separated JSON path for JSON envelopes
        #[arg(long)]
        json_path: Option<String>,

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
