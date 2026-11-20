use crate::contract::Contract;
use crate::engine::check::{CheckReport, Severity};
use crate::engine::diff::{DiffReport, DriftSeverity};
use crate::engine::lint::LintReport;
use crate::error::{KeelError, Result};
use owo_colors::OwoColorize;
use std::path::Path;

pub fn print_check_report(report: &CheckReport, color: bool) -> Result<()> {
    if report.passed {
        let badge = if color {
            " PASS ".on_green().black().bold().to_string()
        } else {
            "[PASS]".to_string()
        };
        println!(
            "{badge} {} — {} rows, {} columns checked (0 errors)",
            report.contract_name, report.rows_checked, report.columns_checked
        );
    } else {
        let badge = if color {
            " FAIL ".on_red().white().bold().to_string()
        } else {
            "[FAIL]".to_string()
        };
        println!(
            "{badge} {} — {} violation(s) across {} rows ({} failing cells)",
            report.contract_name,
            report.violation_count,
            report.rows_checked,
            report.total_failing_cells
        );
    }

    for v in &report.violations {
        let sev = match v.severity {
            Severity::Error => {
                if color {
                    "ERROR".red().bold().to_string()
                } else {
                    "ERROR".to_string()
                }
            }
            Severity::Warning => {
                if color {
                    "WARN ".yellow().bold().to_string()
                } else {
                    "WARN ".to_string()
                }
            }
        };
        let col_part = v
            .column
            .as_deref()
            .map(|c| format!(" [{c}]"))
            .unwrap_or_default();
        println!("\n  {sev} {}{col_part} (count: {})", v.code, v.count);
        println!("    {}", v.message);
        if !v.samples.is_empty() {
            let formatted: Vec<String> = v
                .samples
                .iter()
                .map(|s| match s.row {
                    Some(r) => format!("row {r}: {}", s.value),
                    None => s.value.clone(),
                })
                .collect();
            println!("    samples: {}", formatted.join(" | "));
        }
        if color {
            println!("    {} {}", "hint:".cyan().bold(), v.fix_hint);
        } else {
            println!("    hint: {}", v.fix_hint);
        }
        if let Some(ref cmd) = v.fix_command {
            println!("    run : {cmd}");
        }
    }

    Ok(())
}

pub fn print_diff_report(report: &DiffReport, color: bool) -> Result<()> {
    let badge = if report.failed {
        if color {
            " DRIFT ".on_red().white().bold().to_string()
        } else {
            "[DRIFT]".to_string()
        }
    } else if report.changes.is_empty() {
        if color {
            " CLEAN ".on_green().black().bold().to_string()
        } else {
            "[CLEAN]".to_string()
        }
    } else if color {
        " INFO ".on_yellow().black().bold().to_string()
    } else {
        "[INFO]".to_string()
    };

    let row_info = match (report.baseline_rows, report.current_rows) {
        (Some(b), Some(c)) => format!(" ({b} -> {c} rows)"),
        _ => String::new(),
    };

    println!(
        "{badge} {} -> {}{row_info} | {} breaking, {} warning, {} info",
        report.baseline_path,
        report.current_path,
        report.breaking_count,
        report.warning_count,
        report.info_count
    );

    for ch in &report.changes {
        let sev = match ch.severity {
            DriftSeverity::Breaking => {
                if color {
                    "BREAKING".red().bold().to_string()
                } else {
                    "BREAKING".to_string()
                }
            }
            DriftSeverity::Warning => {
                if color {
                    "WARNING ".yellow().bold().to_string()
                } else {
                    "WARNING ".to_string()
                }
            }
            DriftSeverity::Info => {
                if color {
                    "INFO    ".blue().bold().to_string()
                } else {
                    "INFO    ".to_string()
                }
            }
        };
        println!("\n  {sev} {}", ch.code);
        println!("    {}", ch.message);
        if color {
            println!("    {} {}", "hint:".cyan().bold(), ch.fix_hint);
        } else {
            println!("    hint: {}", ch.fix_hint);
        }
    }

    Ok(())
}

pub fn print_lint_report(report: &LintReport, color: bool) -> Result<()> {
    if report.valid {
        let badge = if color {
            " VALID ".on_green().black().bold().to_string()
        } else {
            "[VALID]".to_string()
        };
        println!(
            "{badge} {} ({} columns defined, {} issue(s))",
            report.contract_path,
            report.columns_defined,
            report.issues.len()
        );
    } else {
        let badge = if color {
            " INVALID ".on_red().white().bold().to_string()
        } else {
            "[INVALID]".to_string()
        };
        println!(
            "{badge} {} ({} issue(s))",
            report.contract_path,
            report.issues.len()
        );
    }

    for issue in &report.issues {
        let sev = match issue.severity {
            Severity::Error => {
                if color {
                    "ERROR".red().bold().to_string()
                } else {
                    "ERROR".to_string()
                }
            }
            Severity::Warning => {
                if color {
                    "WARN ".yellow().bold().to_string()
                } else {
                    "WARN ".to_string()
                }
            }
        };
        println!("\n  {sev} {}", issue.code);
        println!("    {}", issue.message);
        println!("    hint: {}", issue.fix_hint);
    }

    Ok(())
}

pub fn print_infer_saved(
    contract: &Contract,
    output_path: &Path,
    rows: usize,
    color: bool,
) -> Result<()> {
    let badge = if color {
        " SAVED ".on_green().black().bold().to_string()
    } else {
        "[SAVED]".to_string()
    };
    println!(
        "{badge} Inferred contract '{}' ({} columns from {} rows) -> {}",
        contract.name,
        contract.columns.len(),
        rows,
        output_path.display()
    );
    Ok(())
}

pub fn print_codegen_saved(
    contract: &Contract,
    target: &str,
    output_path: &Path,
    color: bool,
) -> Result<()> {
    let badge = if color {
        " GENERATED ".on_green().black().bold().to_string()
    } else {
        "[GENERATED]".to_string()
    };
    println!(
        "{badge} {} code for '{}' -> {}",
        target,
        contract.name,
        output_path.display()
    );
    Ok(())
}

pub fn print_skill_installed(installed_path: &Path, color: bool) -> Result<()> {
    let badge = if color {
        " INSTALLED ".on_green().black().bold().to_string()
    } else {
        "[INSTALLED]".to_string()
    };
    println!("{badge} Agent skill written to {}", installed_path.display());
    Ok(())
}

pub fn print_error(err: &KeelError, color: bool) {
    let badge = if color {
        " ERROR ".on_red().white().bold().to_string()
    } else {
        "[ERROR]".to_string()
    };
    eprintln!("{badge} {}: {err}", err.code());
    if let Some(hint) = err.fix_hint() {
        eprintln!("  hint: {hint}");
    }
}
