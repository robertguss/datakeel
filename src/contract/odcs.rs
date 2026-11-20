use crate::contract::{ColumnContract, Contract, LogicalType};
use crate::error::{KeelError, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const ODCS_API_VERSION: &str = "v3.2.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OdcsDocument {
    pub api_version: String,
    pub kind: String,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<OdcsDescription>,
    #[serde(default)]
    pub schema: Vec<OdcsSchemaObject>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quality: Vec<OdcsQualityCheck>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_properties: Vec<OdcsCustomProperty>,
}

fn default_status() -> String {
    "active".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OdcsDescription {
    Text(String),
    Structured {
        #[serde(skip_serializing_if = "Option::is_none")]
        purpose: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        limitations: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OdcsSchemaObject {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub properties: Vec<OdcsProperty>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quality: Vec<OdcsQualityCheck>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_properties: Vec<OdcsCustomProperty>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OdcsProperty {
    pub name: String,
    #[serde(default = "default_logical_type")]
    pub logical_type: String,
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
    pub logical_type_options: Option<OdcsLogicalTypeOptions>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_properties: Vec<OdcsCustomProperty>,
}

fn default_logical_type() -> String {
    "string".to_string()
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OdcsLogicalTypeOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(rename = "enum", skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
}

impl OdcsLogicalTypeOptions {
    fn is_empty(&self) -> bool {
        self.minimum.is_none()
            && self.maximum.is_none()
            && self.min_length.is_none()
            && self.max_length.is_none()
            && self.pattern.is_none()
            && self.enum_values.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OdcsQualityCheck {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub check_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub must_be_greater_than_or_equal_to: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub must_be_less_than_or_equal_to: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OdcsCustomProperty {
    pub property: String,
    pub value: serde_json::Value,
}

pub fn parse_odcs(content: &str, source_path: &str) -> Result<Contract> {
    let doc: OdcsDocument =
        serde_yaml_ng::from_str(content).map_err(|e| KeelError::ContractParse {
            path: source_path.to_string(),
            detail: e.to_string(),
        })?;

    if !doc.kind.eq_ignore_ascii_case("DataContract") {
        return Err(KeelError::ContractParse {
            path: source_path.to_string(),
            detail: format!("expected kind 'DataContract', found '{}'", doc.kind),
        });
    }

    let obj = doc.schema.first().ok_or_else(|| KeelError::ContractParse {
        path: source_path.to_string(),
        detail: "ODCS contract must contain at least one object in `schema`".to_string(),
    })?;

    let mut strict = false;
    for cp in doc.custom_properties.iter().chain(obj.custom_properties.iter()) {
        if cp.property == "strict"
            && let Some(b) = cp.value.as_bool()
        {
            strict = b;
        }
    }

    let mut min_rows = None;
    let mut max_rows = None;
    for q in doc.quality.iter().chain(obj.quality.iter()) {
        if q.metric.as_deref() == Some("rowCount") {
            if let Some(v) = q.must_be_greater_than_or_equal_to {
                min_rows = Some(v.max(0.0) as usize);
            }
            if let Some(v) = q.must_be_less_than_or_equal_to {
                max_rows = Some(v.max(0.0) as usize);
            }
        }
    }

    let mut columns = IndexMap::new();
    for prop in &obj.properties {
        let mut max_null_rate = None;
        let mut references = None;
        for cp in &prop.custom_properties {
            match cp.property.as_str() {
                "maxNullRate" | "max_null_rate" => {
                    max_null_rate = cp.value.as_f64();
                }
                "references" => {
                    references = cp.value.as_str().map(ToString::to_string);
                }
                _ => {}
            }
        }
        let opts = prop.logical_type_options.as_ref();
        columns.insert(
            prop.name.clone(),
            ColumnContract {
                logical_type: LogicalType::from_odcs_str(&prop.logical_type),
                physical_type: prop.physical_type.clone(),
                required: prop.required || prop.primary_key,
                unique: prop.unique || prop.primary_key,
                primary_key: prop.primary_key,
                description: prop.description.clone(),
                max_null_rate,
                min: opts.and_then(|o| o.minimum),
                max: opts.and_then(|o| o.maximum),
                min_length: opts.and_then(|o| o.min_length),
                max_length: opts.and_then(|o| o.max_length),
                pattern: opts.and_then(|o| o.pattern.clone()),
                allowed_values: opts.and_then(|o| o.enum_values.clone()),
                references,
            },
        );
    }

    let description = match doc.description {
        Some(OdcsDescription::Text(s)) => Some(s),
        Some(OdcsDescription::Structured { purpose, .. }) => purpose,
        None => obj.description.clone(),
    };

    Ok(Contract {
        api_version: doc.api_version,
        id: doc.id,
        name: doc.name,
        version: doc.version,
        status: doc.status,
        description,
        table: obj
            .physical_name
            .clone()
            .unwrap_or_else(|| obj.name.clone()),
        strict,
        min_rows,
        max_rows,
        columns,
    })
}

pub fn serialize_odcs(contract: &Contract) -> Result<String> {
    let mut properties = Vec::with_capacity(contract.columns.len());
    for (col_name, col) in &contract.columns {
        let opts = OdcsLogicalTypeOptions {
            minimum: col.min,
            maximum: col.max,
            min_length: col.min_length,
            max_length: col.max_length,
            pattern: col.pattern.clone(),
            enum_values: col.allowed_values.clone(),
        };
        let mut custom_properties = Vec::new();
        if let Some(rate) = col.max_null_rate {
            custom_properties.push(OdcsCustomProperty {
                property: "maxNullRate".to_string(),
                value: serde_json::json!(rate),
            });
        }
        if let Some(ref fk) = col.references {
            custom_properties.push(OdcsCustomProperty {
                property: "references".to_string(),
                value: serde_json::json!(fk),
            });
        }
        properties.push(OdcsProperty {
            name: col_name.clone(),
            logical_type: col.logical_type.to_string(),
            physical_type: col
                .physical_type
                .clone()
                .or_else(|| Some(col.logical_type.default_postgres_type().to_string())),
            required: col.required,
            unique: col.unique,
            primary_key: col.primary_key,
            description: col.description.clone(),
            logical_type_options: if opts.is_empty() { None } else { Some(opts) },
            custom_properties,
        });
    }

    let mut quality = Vec::new();
    if contract.min_rows.is_some() || contract.max_rows.is_some() {
        quality.push(OdcsQualityCheck {
            check_type: Some("library".to_string()),
            metric: Some("rowCount".to_string()),
            description: Some("Row count bounds check".to_string()),
            must_be_greater_than_or_equal_to: contract.min_rows.map(|v| v as f64),
            must_be_less_than_or_equal_to: contract.max_rows.map(|v| v as f64),
        });
    }

    let mut custom_properties = Vec::new();
    if contract.strict {
        custom_properties.push(OdcsCustomProperty {
            property: "strict".to_string(),
            value: serde_json::json!(true),
        });
    }

    let doc = OdcsDocument {
        api_version: if contract.api_version.is_empty() {
            ODCS_API_VERSION.to_string()
        } else {
            contract.api_version.clone()
        },
        kind: "DataContract".to_string(),
        id: contract.id.clone(),
        name: contract.name.clone(),
        version: contract.version.clone(),
        status: contract.status.clone(),
        description: contract
            .description
            .clone()
            .map(|purpose| OdcsDescription::Structured {
                purpose: Some(purpose),
                usage: None,
                limitations: None,
            }),
        schema: vec![OdcsSchemaObject {
            name: contract.name.clone(),
            physical_name: Some(contract.table.clone()),
            description: None,
            properties,
            quality,
            custom_properties,
        }],
        quality: Vec::new(),
        custom_properties: Vec::new(),
    };

    serde_yaml_ng::to_string(&doc).map_err(|e| KeelError::General(format!("YAML encode error: {e}")))
}
