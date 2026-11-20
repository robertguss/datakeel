pub mod agent;
pub mod human;

use crate::cli::{CodegenTarget, OutputFormatArg};
use crate::contract::Contract;
use crate::engine::check::CheckReport;
use crate::engine::diff::DiffReport;
use crate::engine::lint::LintReport;
use crate::error::{KeelError, Result};
use std::io::IsTerminal;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedFormat {
    Human,
    Agent,
    Json,
}

pub fn resolve_format(arg: OutputFormatArg) -> ResolvedFormat {
    match arg {
        OutputFormatArg::Human => ResolvedFormat::Human,
        OutputFormatArg::Agent => ResolvedFormat::Agent,
        OutputFormatArg::Json => ResolvedFormat::Json,
        OutputFormatArg::Auto => {
            if std::io::stdout().is_terminal() {
                ResolvedFormat::Human
            } else {
                ResolvedFormat::Agent
            }
        }
    }
}

pub fn should_use_color(no_color_flag: bool, format: ResolvedFormat) -> bool {
    if no_color_flag || format != ResolvedFormat::Human {
        return false;
    }
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::stdout().is_terminal()
}

pub fn emit_check_report(
    report: &CheckReport,
    format: ResolvedFormat,
    use_color: bool,
) -> Result<()> {
    match format {
        ResolvedFormat::Human => human::print_check_report(report, use_color),
        ResolvedFormat::Agent => agent::print_check_report(report),
        ResolvedFormat::Json => print_json(report),
    }
}

pub fn emit_diff_report(
    report: &DiffReport,
    format: ResolvedFormat,
    use_color: bool,
) -> Result<()> {
    match format {
        ResolvedFormat::Human => human::print_diff_report(report, use_color),
        ResolvedFormat::Agent => agent::print_diff_report(report),
        ResolvedFormat::Json => print_json(report),
    }
}

pub fn emit_lint_report(
    report: &LintReport,
    format: ResolvedFormat,
    use_color: bool,
) -> Result<()> {
    match format {
        ResolvedFormat::Human => human::print_lint_report(report, use_color),
        ResolvedFormat::Agent => agent::print_lint_report(report),
        ResolvedFormat::Json => print_json(report),
    }
}

pub fn emit_infer_saved(
    contract: &Contract,
    output_path: &Path,
    rows: usize,
    format: ResolvedFormat,
    use_color: bool,
) -> Result<()> {
    match format {
        ResolvedFormat::Human => human::print_infer_saved(contract, output_path, rows, use_color),
        ResolvedFormat::Agent | ResolvedFormat::Json => print_json(&serde_json::json!({
            "schema_version": "1.0",
            "command": "infer",
            "ok": true,
            "contract_name": contract.name,
            "output_path": output_path.display().to_string(),
            "rows_profiled": rows,
            "columns_inferred": contract.columns.len(),
        })),
    }
}

pub fn emit_codegen_saved(
    contract: &Contract,
    target: CodegenTarget,
    output_path: &Path,
    format: ResolvedFormat,
    use_color: bool,
) -> Result<()> {
    let target_str = match target {
        CodegenTarget::Postgres => "postgres",
        CodegenTarget::Pydantic => "pydantic",
    };
    match format {
        ResolvedFormat::Human => {
            human::print_codegen_saved(contract, target_str, output_path, use_color)
        }
        ResolvedFormat::Agent | ResolvedFormat::Json => print_json(&serde_json::json!({
            "schema_version": "1.0",
            "command": "codegen",
            "ok": true,
            "contract_name": contract.name,
            "target": target_str,
            "output_path": output_path.display().to_string(),
        })),
    }
}

pub fn emit_skill_installed(
    installed_path: &Path,
    format: ResolvedFormat,
    use_color: bool,
) -> Result<()> {
    match format {
        ResolvedFormat::Human => human::print_skill_installed(installed_path, use_color),
        ResolvedFormat::Agent | ResolvedFormat::Json => print_json(&serde_json::json!({
            "schema_version": "1.0",
            "command": "skill",
            "ok": true,
            "installed_path": installed_path.display().to_string(),
        })),
    }
}

pub fn emit_error(err: &KeelError, format: ResolvedFormat, use_color: bool) {
    match format {
        ResolvedFormat::Human => human::print_error(err, use_color),
        ResolvedFormat::Agent | ResolvedFormat::Json => {
            let envelope = err.to_envelope();
            if let Ok(json) = serde_json::to_string_pretty(&envelope) {
                eprintln!("{json}");
            } else {
                eprintln!("{{\"ok\":false,\"error\":{{\"code\":\"{}\"}}}}", err.code());
            }
        }
    }
}

fn print_json<T: serde::Serialize>(val: &T) -> Result<()> {
    let s = serde_json::to_string_pretty(val)
        .map_err(|e| KeelError::General(format!("JSON serialization error: {e}")))?;
    println!("{s}");
    Ok(())
}
