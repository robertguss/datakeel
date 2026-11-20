use crate::contract::odcs::ODCS_API_VERSION;
use crate::contract::{ColumnContract, Contract, LogicalType};
use crate::error::{KeelError, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TomlContract {
    pub contract: TomlHeader,
    #[serde(default)]
    pub columns: IndexMap<String, TomlColumn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TomlHeader {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub strict: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_rows: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rows: Option<usize>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TomlColumn {
    #[serde(rename = "type")]
    pub logical_type: LogicalType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_type: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub unique: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub primary_key: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
    #[serde(rename = "enum", skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub references: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

pub fn parse_toml(content: &str, source_path: &str) -> Result<Contract> {
    let raw: TomlContract = toml::from_str(content).map_err(|e| KeelError::ContractParse {
        path: source_path.to_string(),
        detail: e.to_string(),
    })?;

    let mut columns = IndexMap::new();
    for (col_name, col) in raw.columns {
        columns.insert(
            col_name,
            ColumnContract {
                logical_type: col.logical_type,
                physical_type: col.physical_type,
                required: col.required || col.primary_key,
                unique: col.unique || col.primary_key,
                primary_key: col.primary_key,
                description: col.description,
                max_null_rate: col.max_null_rate,
                min: col.min,
                max: col.max,
                min_length: col.min_length,
                max_length: col.max_length,
                pattern: col.pattern,
                allowed_values: col.allowed_values,
                references: col.references,
            },
        );
    }

    let table = raw
        .contract
        .table
        .clone()
        .unwrap_or_else(|| raw.contract.name.clone());

    Ok(Contract {
        api_version: ODCS_API_VERSION.to_string(),
        id: format!("urn:datakeel:{}", raw.contract.name),
        name: raw.contract.name,
        version: raw.contract.version,
        status: "active".to_string(),
        description: raw.contract.description,
        table,
        strict: raw.contract.strict,
        min_rows: raw.contract.min_rows,
        max_rows: raw.contract.max_rows,
        columns,
    })
}

pub fn serialize_toml(contract: &Contract) -> Result<String> {
    let mut columns = IndexMap::new();
    for (name, col) in &contract.columns {
        columns.insert(
            name.clone(),
            TomlColumn {
                logical_type: col.logical_type,
                physical_type: col.physical_type.clone(),
                required: col.required,
                unique: col.unique,
                primary_key: col.primary_key,
                description: col.description.clone(),
                max_null_rate: col.max_null_rate,
                min: col.min,
                max: col.max,
                min_length: col.min_length,
                max_length: col.max_length,
                pattern: col.pattern.clone(),
                allowed_values: col.allowed_values.clone(),
                references: col.references.clone(),
            },
        );
    }

    let doc = TomlContract {
        contract: TomlHeader {
            name: contract.name.clone(),
            version: contract.version.clone(),
            table: if contract.table != contract.name {
                Some(contract.table.clone())
            } else {
                None
            },
            description: contract.description.clone(),
            strict: contract.strict,
            min_rows: contract.min_rows,
            max_rows: contract.max_rows,
        },
        columns,
    };

    toml::to_string_pretty(&doc).map_err(|e| KeelError::General(format!("TOML encode error: {e}")))
}
