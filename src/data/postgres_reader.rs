use crate::data::json::json_object_to_row;
use crate::data::{Dataset, LoadOptions};
use crate::error::{KeelError, Result};
use indexmap::IndexSet;
use native_tls::TlsConnector;
use postgres::{Client, NoTls};
use postgres_native_tls::MakeTlsConnector;
use serde_json::Value as JsonValue;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgTarget {
    /// Full libpq/URL connection string (with datakeel-only query params stripped).
    pub conn_url: String,
    /// Safe display label with password redacted (never leaks secrets to stdout/stderr/contracts).
    pub safe_display: String,
    pub schema: String,
    pub table: String,
    pub where_clause: Option<String>,
    pub limit: Option<usize>,
    pub sslmode: SslMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SslMode {
    Disable,
    Prefer,
    Require,
    Verify,
}

pub fn is_postgres_uri(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    lower.starts_with("postgres://")
        || lower.starts_with("postgresql://")
        || lower.starts_with("pg://")
}

/// Extract the bare table name from a Postgres URI (for deriving default contract names).
pub fn extract_table_name_hint(input: &str, explicit_table: Option<&str>) -> Option<String> {
    if let Some(t) = explicit_table {
        let (_, tbl) = split_schema_table(t);
        return Some(tbl);
    }
    let lower = input.to_ascii_lowercase();
    if lower.starts_with("pg://") {
        let rest = &input[5..];
        let table_part = rest.split('?').next().unwrap_or("").trim_matches('/');
        if !table_part.is_empty() {
            let (_, tbl) = split_schema_table(table_part);
            return Some(tbl);
        }
    } else if let Ok(url) = Url::parse(input) {
        for (k, v) in url.query_pairs() {
            if k == "table" && !v.trim().is_empty() {
                let (_, tbl) = split_schema_table(&v);
                return Some(tbl);
            }
        }
    }
    None
}

pub fn resolve_pg_target(input: &str, opts: &LoadOptions) -> Result<PgTarget> {
    let lower = input.to_ascii_lowercase();
    let (raw_conn_url, shorthand_table, extra_query_str) = if lower.starts_with("pg://") {
        let rest = &input[5..];
        let (path_part, query_part) = match rest.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (rest, None),
        };
        let table_from_shorthand = path_part.trim_matches('/').to_string();
        let env_url = std::env::var("DATAKEEL_DATABASE_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                std::env::var("DATABASE_URL")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            })
            .ok_or_else(|| KeelError::PgMissingDatabaseUrl {
                target: input.to_string(),
            })?;
        (
            env_url,
            if table_from_shorthand.is_empty() {
                None
            } else {
                Some(table_from_shorthand)
            },
            query_part,
        )
    } else {
        (input.to_string(), None, None)
    };

    let mut url = Url::parse(&raw_conn_url).map_err(|e| KeelError::PgConnect {
        target: redact_uri(&raw_conn_url),
        detail: format!("invalid Postgres connection URL: {e}"),
    })?;

    let mut table_param = shorthand_table;
    let mut where_param: Option<String> = None;
    let mut limit_param: Option<usize> = None;
    let mut sslmode = SslMode::Prefer;

    // Parse query params from the connection URL (and strip datakeel-specific params before passing to libpq)
    let mut kept_pairs: Vec<(String, String)> = Vec::new();
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "table" if !v.trim().is_empty() => {
                table_param = Some(v.into_owned());
            }
            "where" if !v.trim().is_empty() => {
                where_param = Some(v.into_owned());
            }
            "limit" => {
                if let Ok(n) = v.parse::<usize>() {
                    limit_param = Some(n);
                }
            }
            "sslmode" => {
                sslmode = parse_sslmode(&v);
                kept_pairs.push((k.into_owned(), v.into_owned()));
            }
            _ => kept_pairs.push((k.into_owned(), v.into_owned())),
        }
    }

    // Also parse query params on `pg://table?limit=100&where=...`
    if let Some(q) = extra_query_str {
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                let val = percent_decode_simple(v);
                match k {
                    "where" if !val.trim().is_empty() => where_param = Some(val),
                    "limit" => {
                        if let Ok(n) = val.parse::<usize>() {
                            limit_param = Some(n);
                        }
                    }
                    "sslmode" => sslmode = parse_sslmode(&val),
                    _ => {}
                }
            }
        }
    }

    // CLI flags override URI query params
    if let Some(ref t) = opts.table {
        table_param = Some(t.clone());
    }
    if let Some(ref w) = opts.where_clause {
        where_param = Some(w.clone());
    }
    if let Some(l) = opts.limit {
        limit_param = Some(l);
    }

    // Rebuild clean conn URL without datakeel-only query parameters
    if kept_pairs.is_empty() {
        url.set_query(None);
    } else {
        let mut ser = url.query_pairs_mut();
        ser.clear();
        for (k, v) in &kept_pairs {
            ser.append_pair(k, v);
        }
    }

    let safe_base = redact_url(&url);
    let table_spec = table_param.ok_or_else(|| KeelError::PgMissingTable {
        target: safe_base.clone(),
    })?;
    let (schema, table) = split_schema_table(&table_spec);
    if table.is_empty() {
        return Err(KeelError::PgMissingTable { target: safe_base });
    }

    let safe_display = format!("{safe_base}?table={schema}.{table}");

    Ok(PgTarget {
        conn_url: url.to_string(),
        safe_display,
        schema,
        table,
        where_clause: where_param,
        limit: limit_param,
        sslmode,
    })
}

pub fn read_postgres(input: &str, opts: &LoadOptions) -> Result<Dataset> {
    let target = resolve_pg_target(input, opts)?;
    let mut client = connect_postgres(&target)?;

    // 1. Introspect columns in ordinal order from information_schema.columns
    let col_rows = client
        .query(
            "SELECT column_name \
             FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 \
             ORDER BY ordinal_position",
            &[&target.schema, &target.table],
        )
        .map_err(|e| KeelError::PgQuery {
            target: target.safe_display.clone(),
            detail: e.to_string(),
        })?;

    if col_rows.is_empty() {
        return Err(KeelError::PgTableNotFound {
            target: target.safe_display.clone(),
            table: format!("{}.{}", target.schema, target.table),
        });
    }

    let mut columns = IndexSet::with_capacity(col_rows.len());
    for r in &col_rows {
        let col_name: String = r.get(0);
        columns.insert(col_name);
    }

    // 2. Fetch rows via row_to_json(t)::text so ALL Postgres types (NUMERIC, UUID, ENUM, JSONB, arrays, etc.) decode safely
    let qualified = format!("{}.{}", quote_ident(&target.schema), quote_ident(&target.table));
    let mut inner = format!("SELECT * FROM {qualified}");
    if let Some(ref pred) = target.where_clause {
        inner.push_str(" WHERE ");
        inner.push_str(pred);
    }
    if let Some(lim) = target.limit {
        inner.push_str(&format!(" LIMIT {lim}"));
    }
    let sql = format!("SELECT row_to_json(t)::text FROM ({inner}) t");

    let pg_rows = client.query(&sql, &[]).map_err(|e| KeelError::PgQuery {
        target: target.safe_display.clone(),
        detail: e.to_string(),
    })?;

    let mut rows = Vec::with_capacity(pg_rows.len());
    for (idx, pg_row) in pg_rows.iter().enumerate() {
        let raw_json: String = pg_row.get(0);
        let parsed: JsonValue =
            serde_json::from_str(&raw_json).map_err(|e| KeelError::DataParse {
                path: target.safe_display.clone(),
                detail: format!("row {}: {e}", idx + 1),
            })?;
        rows.push(json_object_to_row(&parsed, &target.safe_display, idx + 1)?);
    }

    Ok(Dataset {
        source_name: target.safe_display,
        columns,
        rows,
    })
}

fn connect_postgres(target: &PgTarget) -> Result<Client> {
    match target.sslmode {
        SslMode::Disable => Client::connect(&target.conn_url, NoTls).map_err(|e| {
            KeelError::PgConnect {
                target: target.safe_display.clone(),
                detail: e.to_string(),
            }
        }),
        SslMode::Prefer => {
            // Try TLS first; fall back to NoTls if server doesn't offer TLS (common for local Postgres).
            if let Ok(connector) = make_tls(false)
                && let Ok(client) = Client::connect(&target.conn_url, connector)
            {
                return Ok(client);
            }
            Client::connect(&target.conn_url, NoTls).map_err(|e| KeelError::PgConnect {
                target: target.safe_display.clone(),
                detail: e.to_string(),
            })
        }
        SslMode::Require => {
            let connector = make_tls(false).map_err(|e| KeelError::PgConnect {
                target: target.safe_display.clone(),
                detail: e,
            })?;
            Client::connect(&target.conn_url, connector).map_err(|e| KeelError::PgConnect {
                target: target.safe_display.clone(),
                detail: e.to_string(),
            })
        }
        SslMode::Verify => {
            let connector = make_tls(true).map_err(|e| KeelError::PgConnect {
                target: target.safe_display.clone(),
                detail: e,
            })?;
            Client::connect(&target.conn_url, connector).map_err(|e| KeelError::PgConnect {
                target: target.safe_display.clone(),
                detail: e.to_string(),
            })
        }
    }
}

fn make_tls(verify_certs: bool) -> std::result::Result<MakeTlsConnector, String> {
    let mut builder = TlsConnector::builder();
    if !verify_certs {
        builder.danger_accept_invalid_certs(true);
        builder.danger_accept_invalid_hostnames(true);
    }
    let tls = builder.build().map_err(|e| e.to_string())?;
    Ok(MakeTlsConnector::new(tls))
}

fn parse_sslmode(s: &str) -> SslMode {
    match s.trim().to_ascii_lowercase().as_str() {
        "disable" => SslMode::Disable,
        "require" => SslMode::Require,
        "verify-ca" | "verify-full" => SslMode::Verify,
        _ => SslMode::Prefer,
    }
}

fn split_schema_table(spec: &str) -> (String, String) {
    let cleaned = spec.trim();
    match cleaned.split_once('.') {
        Some((s, t)) => (s.trim().to_string(), t.trim().to_string()),
        None => ("public".to_string(), cleaned.to_string()),
    }
}

fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

pub fn redact_uri(raw: &str) -> String {
    match Url::parse(raw) {
        Ok(u) => redact_url(&u),
        Err(_) => "<postgres-uri>".to_string(),
    }
}

fn redact_url(url: &Url) -> String {
    let mut cloned = url.clone();
    if cloned.password().is_some() {
        let _ = cloned.set_password(Some("***"));
    }
    cloned.to_string()
}

fn percent_decode_simple(s: &str) -> String {
    let replaced = s.replace('+', " ");
    let bytes = replaced.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3])
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_full_postgres_uri_and_redacts_password() {
        let target = resolve_pg_target(
            "postgres://alice:supersecret@db.example.com:5432/seminary?table=staging.populi_students&limit=500&sslmode=require",
            &LoadOptions::default(),
        )
        .unwrap();

        assert_eq!(target.schema, "staging");
        assert_eq!(target.table, "populi_students");
        assert_eq!(target.limit, Some(500));
        assert_eq!(target.sslmode, SslMode::Require);
        assert!(!target.safe_display.contains("supersecret"));
        assert!(target.safe_display.contains("***"));
        assert!(!target.conn_url.contains("table="));
        assert!(target.conn_url.contains("sslmode=require"));
    }

    #[test]
    fn cli_table_and_where_override_uri_params() {
        let opts = LoadOptions {
            json_path: None,
            table: Some("analytics.risk_scores".to_string()),
            where_clause: Some("gpa < 2.5".to_string()),
            limit: Some(100),
        };
        let target =
            resolve_pg_target("postgres://localhost:5432/seminary?sslmode=disable", &opts).unwrap();

        assert_eq!(target.schema, "analytics");
        assert_eq!(target.table, "risk_scores");
        assert_eq!(target.where_clause.as_deref(), Some("gpa < 2.5"));
        assert_eq!(target.limit, Some(100));
        assert_eq!(target.sslmode, SslMode::Disable);
    }

    #[test]
    fn extract_table_name_hint_handles_shorthand_and_full_uri() {
        assert_eq!(
            extract_table_name_hint("pg://staging.populi_students", None).as_deref(),
            Some("populi_students")
        );
        assert_eq!(
            extract_table_name_hint(
                "postgres://localhost/db?table=public.canvas_enrollments",
                None
            )
            .as_deref(),
            Some("canvas_enrollments")
        );
    }
}
