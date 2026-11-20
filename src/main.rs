use clap::Parser;
use datakeel::cli::{Cli, Commands, ContractFormatArg, DataFormatArg, OutputFormatArg};
use datakeel::contract::{Contract, ContractFormat};
use datakeel::data::{LoadOptions, load_dataset, postgres_reader};
use datakeel::engine::{check, codegen, diff, infer, lint};
use datakeel::error::{EXIT_ERROR, EXIT_OK, EXIT_VIOLATION, KeelError, Result};
use datakeel::output::{self, ResolvedFormat};
use datakeel::skill;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = output::resolve_format(cli.format);
    let use_color = output::should_use_color(cli.no_color, format);

    match run(cli, format, use_color) {
        Ok(code) => ExitCode::from(code as u8),
        Err(err) => {
            output::emit_error(&err, format, use_color);
            ExitCode::from(EXIT_ERROR as u8)
        }
    }
}

fn run(cli: Cli, format: ResolvedFormat, use_color: bool) -> Result<i32> {
    match cli.command {
        Commands::Infer {
            input,
            input_format,
            json_path,
            table,
            where_clause,
            limit,
            contract_format,
            name,
            infer_enums,
            output: out_path,
        } => {
            let load_opts = LoadOptions {
                json_path,
                table: table.clone(),
                where_clause,
                limit,
            };
            let dataset = load_dataset(&input, input_format, &load_opts)?;
            let table_name = name.unwrap_or_else(|| derive_table_name(&input, table.as_deref()));
            let contract = infer::infer_contract(
                &dataset,
                &table_name,
                infer::InferOptions {
                    max_enum_cardinality: infer_enums,
                },
            );

            let fmt = match contract_format {
                ContractFormatArg::Odcs => ContractFormat::OdcsYaml,
                ContractFormatArg::Toml => ContractFormat::Toml,
            };
            let rendered = contract.serialize(fmt)?;

            if let Some(path) = out_path {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                {
                    fs::create_dir_all(parent).map_err(|source| KeelError::IoWrite {
                        path: path.clone(),
                        source,
                    })?;
                }
                fs::write(&path, &rendered).map_err(|source| KeelError::IoWrite {
                    path: path.clone(),
                    source,
                })?;
                output::emit_infer_saved(&contract, &path, dataset.row_count(), format, use_color)?;
            } else {
                // Pure contract content on stdout; no noise mixed in.
                print!("{rendered}");
            }
            Ok(EXIT_OK)
        }

        Commands::Check {
            contract: contract_path,
            data,
            input_format,
            json_path,
            table,
            where_clause,
            limit,
            refs,
            strict,
            max_samples,
        } => {
            let contract = Contract::load_from_path(&contract_path)?;
            let load_opts = LoadOptions {
                json_path,
                table,
                where_clause,
                limit,
            };
            let dataset = load_dataset(&data, input_format, &load_opts)?;

            let mut ref_datasets = HashMap::new();
            for raw in refs {
                let (ref_name, ref_source) =
                    raw.split_once('=').ok_or_else(|| KeelError::InvalidRefArg {
                        raw: raw.clone(),
                    })?;
                if ref_name.trim().is_empty() || ref_source.trim().is_empty() {
                    return Err(KeelError::InvalidRefArg { raw });
                }
                let ref_ds = load_dataset(ref_source, DataFormatArg::Auto, &LoadOptions::default())?;
                ref_datasets.insert(ref_name.trim().to_string(), ref_ds);
            }

            let report = check::check_dataset(
                &contract,
                &dataset,
                &check::CheckOptions {
                    strict_override: strict,
                    max_samples,
                    contract_path: contract_path.display().to_string(),
                    data_path: dataset.source_name.clone(),
                },
                &ref_datasets,
            )?;

            output::emit_check_report(&report, format, use_color)?;
            if report.passed {
                Ok(EXIT_OK)
            } else {
                Ok(EXIT_VIOLATION)
            }
        }

        Commands::Diff {
            baseline,
            current,
            baseline_format,
            current_format,
            json_path,
            where_clause,
            limit,
            max_null_delta,
            max_row_drop_pct,
            fail_on_any,
        } => {
            let report = diff::diff_sources(
                &baseline,
                &current,
                diff::DiffOptions {
                    baseline_format,
                    current_format,
                    json_path,
                    where_clause,
                    limit,
                    max_null_delta,
                    max_row_drop_pct,
                    fail_on_any,
                },
            )?;
            output::emit_diff_report(&report, format, use_color)?;
            if report.failed {
                Ok(EXIT_VIOLATION)
            } else {
                Ok(EXIT_OK)
            }
        }

        Commands::Codegen {
            contract: contract_path,
            target,
            schema,
            output: out_path,
        } => {
            let contract = Contract::load_from_path(&contract_path)?;
            let code = codegen::generate(&contract, target, schema.as_deref());

            if let Some(path) = out_path {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                {
                    fs::create_dir_all(parent).map_err(|source| KeelError::IoWrite {
                        path: path.clone(),
                        source,
                    })?;
                }
                fs::write(&path, &code).map_err(|source| KeelError::IoWrite {
                    path: path.clone(),
                    source,
                })?;
                output::emit_codegen_saved(&contract, target, &path, format, use_color)?;
            } else {
                print!("{code}");
            }
            Ok(EXIT_OK)
        }

        Commands::Lint {
            contract: contract_path,
        } => {
            let report = lint::lint_contract_file(&contract_path)?;
            output::emit_lint_report(&report, format, use_color)?;
            if report.valid {
                Ok(EXIT_OK)
            } else {
                Ok(EXIT_VIOLATION)
            }
        }

        Commands::Skill { install } => {
            if let Some(dir) = install {
                let installed_path = skill::install_skill(&dir)?;
                output::emit_skill_installed(&installed_path, format, use_color)?;
            } else {
                print!("{}", skill::SKILL_MD);
            }
            Ok(EXIT_OK)
        }
    }
}

fn derive_table_name(input: &str, explicit_table: Option<&str>) -> String {
    if let Some(pg_table) = postgres_reader::extract_table_name_hint(input, explicit_table) {
        return sanitize_ident(&pg_table);
    }
    if input == "-" {
        return "stdin_dataset".to_string();
    }
    let path = PathBuf::from(input);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("dataset");
    sanitize_ident(stem)
}

fn sanitize_ident(raw: &str) -> String {
    let clean: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() {
        "dataset".to_string()
    } else {
        clean
    }
}

// Keep unused import check satisfied in case OutputFormatArg is referenced cleanly
#[allow(dead_code)]
fn _assert_format(_: OutputFormatArg) {}
