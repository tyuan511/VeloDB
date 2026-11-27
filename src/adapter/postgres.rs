use super::*;
use deadpool_postgres::{Config as PoolConfig, ManagerConfig, Pool, RecyclingMethod, Runtime};
use futures_util::TryStreamExt;
use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use std::time::Instant;
use tokio_postgres::types::{FromSql, FromSqlOwned, Kind, ToSql, Type};
use tokio_postgres::{error::SqlState, Column, NoTls, Row, RowStream};

fn query_error(error: tokio_postgres::Error) -> AdapterError {
    let message = match error.as_db_error() {
        Some(db) => format!("[{}] {db}", db.code().code()),
        None => error.to_string(),
    };
    AdapterError::Query(message)
}

pub struct PostgresAdapter {
    config: ConnectionConfig,
    pool: Pool,
}

impl PostgresAdapter {
    pub fn new(config: ConnectionConfig) -> AdapterResult<Self> {
        let mut pg_cfg = PoolConfig::new();
        pg_cfg.host = Some(config.host.clone());
        pg_cfg.port = Some(config.port);
        pg_cfg.dbname = Some(if config.database.trim().is_empty() {
            "postgres".to_string()
        } else {
            config.database.clone()
        });
        pg_cfg.user = Some(config.username.clone());
        if !config.password.is_empty() {
            pg_cfg.password = Some(config.password.clone());
        }
        pg_cfg.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });

        let pool = match config.ssl_mode {
            SslMode::Disable => pg_cfg.create_pool(Some(Runtime::Tokio1), NoTls),
            SslMode::Prefer | SslMode::Require => {
                let tls = TlsConnector::builder()
                    .danger_accept_invalid_certs(true)
                    .build()
                    .map_err(|e| AdapterError::Connection(format!("TLS init failed: {e}")))?;
                pg_cfg.create_pool(Some(Runtime::Tokio1), MakeTlsConnector::new(tls))
            }
            SslMode::VerifyFull => {
                let tls = TlsConnector::builder()
                    .build()
                    .map_err(|e| AdapterError::Connection(format!("TLS init failed: {e}")))?;
                pg_cfg.create_pool(Some(Runtime::Tokio1), MakeTlsConnector::new(tls))
            }
        }
        .map_err(|e| AdapterError::Connection(format!("Failed to create pool: {e}")))?;

        Ok(Self { config, pool })
    }

    async fn get_client(&self) -> AdapterResult<deadpool_postgres::Object> {
        self.pool
            .get()
            .await
            .map_err(|e| AdapterError::Connection(e.to_string()))
    }

    /// Resolve result types before requesting any rows. A one-shot typed query
    /// can block fetching an unknown type behind its own pending data.
    async fn page_stream(
        client: &deadpool_postgres::Object,
        mut sql: String,
        query: &TableDataQuery,
    ) -> AdapterResult<RowStream> {
        let limit = query.limit.saturating_add(1);
        // Keep user-written predicates independent of our pagination parameters.
        // In particular, a literal or reference containing `$1` keeps its meaning.
        if query
            .where_clause
            .as_deref()
            .is_some_and(|w| w.contains('$'))
        {
            sql.push_str(&format!(" LIMIT {limit} OFFSET {}", query.offset));
            return client
                .query_raw(&sql, std::iter::empty::<&(dyn ToSql + Sync)>())
                .await
                .map_err(query_error);
        }

        let limit =
            i64::try_from(limit).map_err(|_| AdapterError::Query("LIMIT exceeds bigint".into()))?;
        let offset = i64::try_from(query.offset)
            .map_err(|_| AdapterError::Query("OFFSET exceeds bigint".into()))?;
        sql.push_str(" LIMIT $1 OFFSET $2");
        let params: [&(dyn ToSql + Sync); 2] = [&limit, &offset];
        let types = [Type::INT8, Type::INT8];
        // Bound the number of distinct browse statements retained per connection.
        if client.statement_cache.size() >= 32 {
            client.statement_cache.clear();
        }
        let stmt = client
            .prepare_typed_cached(&sql, &types)
            .await
            .map_err(query_error)?;
        match client.query_raw(&stmt, params).await {
            // The server can invalidate a prepared result shape after ALTER TABLE,
            // or forget a statement after DEALLOCATE. A bind failure precedes execution.
            Err(e)
                if matches!(
                    e.code(),
                    Some(&SqlState::FEATURE_NOT_SUPPORTED | &SqlState::INVALID_SQL_STATEMENT_NAME)
                ) =>
            {
                client.statement_cache.remove(&sql, &types);
                let stmt = client
                    .prepare_typed_cached(&sql, &types)
                    .await
                    .map_err(query_error)?;
                client.query_raw(&stmt, params).await.map_err(query_error)
            }
            result => result.map_err(query_error),
        }
    }
}

pub fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

pub fn sql_literal(val: &Option<String>) -> String {
    match val {
        None => "NULL".to_string(),
        Some(s) => format!("'{}'", s.replace('\'', "''")),
    }
}

pub fn generate_staged_sql(changes: &[StagedChange]) -> String {
    let mut lines = vec!["BEGIN;".to_string()];
    for ch in changes {
        let fq = format!("{}.{}", quote_ident(&ch.schema), quote_ident(&ch.table));
        match &ch.kind {
            StagedChangeKind::Update { col, new_val, .. } => {
                let pk_cond = match &ch.pk_val {
                    Some(v) => format!("{} = {}", quote_ident(&ch.pk_col), sql_literal(&Some(v.clone()))),
                    None => "ctid = NULL /* missing PK */".to_string(),
                };
                lines.push(format!(
                    "UPDATE {fq} SET {} = {} WHERE {pk_cond};",
                    quote_ident(col),
                    sql_literal(new_val)
                ));
            }
            StagedChangeKind::Insert { values } => {
                if values.is_empty() {
                    lines.push(format!("INSERT INTO {fq} DEFAULT VALUES;"));
                } else {
                    let cols: Vec<String> = values.keys().map(|k| quote_ident(k)).collect();
                    let vals: Vec<String> = values.values().map(sql_literal).collect();
                    lines.push(format!(
                        "INSERT INTO {fq} ({}) VALUES ({});",
                        cols.join(", "),
                        vals.join(", ")
                    ));
                }
            }
            StagedChangeKind::Delete => {
                let pk_cond = match &ch.pk_val {
                    Some(v) => format!("{} = {}", quote_ident(&ch.pk_col), sql_literal(&Some(v.clone()))),
                    None => "FALSE".to_string(),
                };
                lines.push(format!("DELETE FROM {fq} WHERE {pk_cond};"));
            }
        }
    }
    lines.push("COMMIT;".to_string());
    lines.join("\n")
}

pub fn export_rows(
    columns: &[String],
    rows: &[Vec<Option<String>>],
    schema: &str,
    table: &str,
    fmt: ExportFormat,
) -> String {
    match fmt {
        ExportFormat::Csv => {
            let mut out = Vec::with_capacity(rows.len() + 1);
            out.push(columns.join(","));
            for r in rows {
                let line: Vec<String> = r
                    .iter()
                    .map(|cell| match cell {
                        None => String::new(),
                        Some(s) => {
                            if s.contains(',') || s.contains('"') || s.contains('\n') {
                                format!("\"{}\"", s.replace('"', "\"\""))
                            } else {
                                s.clone()
                            }
                        }
                    })
                    .collect();
                out.push(line.join(","));
            }
            out.join("\n")
        }
        ExportFormat::Json => {
            let json_rows: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    let mut map = serde_json::Map::new();
                    for (idx, col) in columns.iter().enumerate() {
                        let val = r.get(idx).and_then(|v| v.as_ref());
                        let jv = match val {
                            None => serde_json::Value::Null,
                            Some(s) => serde_json::Value::String(s.clone()),
                        };
                        map.insert(col.clone(), jv);
                    }
                    serde_json::Value::Object(map)
                })
                .collect();
            serde_json::to_string_pretty(&json_rows).unwrap_or_else(|_| "[]".to_string())
        }
        ExportFormat::SqlInsert => {
            let fq = format!("{}.{}", quote_ident(schema), quote_ident(table));
            let col_list = columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ");
            rows.iter()
                .map(|r| {
                    let vals = r.iter().map(sql_literal).collect::<Vec<_>>().join(", ");
                    format!("INSERT INTO {fq} ({col_list}) VALUES ({vals});")
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

pub fn format_sql(sql: &str, uppercase_keywords: bool, indent_spaces: u8) -> String {
    let options = sqlformat::FormatOptions {
        indent: sqlformat::Indent::Spaces(indent_spaces),
        uppercase: Some(uppercase_keywords),
        lines_between_queries: 2,
        ..Default::default()
    };
    sqlformat::format(sql, &sqlformat::QueryParams::None, &options)
}

pub fn is_read_only_sql(sql: &str) -> bool {
    let trimmed = sql.trim_start();
    let upper = trimmed
        .lines()
        .filter(|l| !l.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join(" ")
        .trim_start()
        .to_uppercase();
    upper.starts_with("SELECT")
        || upper.starts_with("WITH")
        || upper.starts_with("EXPLAIN")
        || upper.starts_with("SHOW")
        || upper.starts_with("VALUES")
}

fn format_bytea(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(2 + 2 * bytes.len());
    text.push_str("\\x");
    for &byte in bytes {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 0xf) as usize] as char);
    }
    text
}

type CellDecoder = fn(&Row, usize) -> Option<String>;

fn display_cell<T: FromSqlOwned + std::fmt::Display>(row: &Row, index: usize) -> Option<String> {
    row.try_get::<_, Option<T>>(index)
        .ok()
        .flatten()
        .map(|v| v.to_string())
}

// PostgreSQL sends an enum's UTF-8 label in binary results as well.
struct EnumLabel(String);

impl<'a> FromSql<'a> for EnumLabel {
    fn accepts(ty: &Type) -> bool {
        matches!(ty.kind(), Kind::Enum(_))
    }

    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(std::str::from_utf8(raw)?.to_owned()))
    }
}

fn cell_decoder(ty: &Type) -> CellDecoder {
    match *ty {
        Type::BOOL => display_cell::<bool>,
        Type::INT2 => display_cell::<i16>,
        Type::INT4 => display_cell::<i32>,
        Type::INT8 => display_cell::<i64>,
        Type::OID => display_cell::<u32>,
        Type::FLOAT4 => display_cell::<f32>,
        Type::FLOAT8 => display_cell::<f64>,
        Type::UUID => display_cell::<uuid::Uuid>,
        Type::JSON | Type::JSONB => display_cell::<serde_json::Value>,
        Type::DATE => display_cell::<chrono::NaiveDate>,
        Type::TIMESTAMPTZ => |r, i| {
            r.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(i)
                .ok()
                .flatten()
                .map(|v| v.format("%Y-%m-%d %H:%M:%S+00").to_string())
        },
        Type::TIMESTAMP => |r, i| {
            r.try_get::<_, Option<chrono::NaiveDateTime>>(i)
                .ok()
                .flatten()
                .map(|v| v.format("%Y-%m-%d %H:%M:%S").to_string())
        },
        Type::BYTEA => |r, i| {
            r.try_get::<_, Option<&[u8]>>(i)
                .ok()
                .flatten()
                .map(format_bytea)
        },
        _ if <String as FromSql>::accepts(ty) => |r, i| r.try_get::<_, Option<String>>(i).ok().flatten(),
        _ if EnumLabel::accepts(ty) => |r, i| {
            r.try_get::<_, Option<EnumLabel>>(i)
                .ok()
                .flatten()
                .map(|v| v.0)
        },
        _ if <Vec<String> as FromSql>::accepts(ty) => |r, i| {
            r.try_get::<_, Option<Vec<String>>>(i)
                .ok()
                .flatten()
                .map(|v| format!("{{{}}}", v.join(",")))
        },
        _ => |_, _| None,
    }
}

fn column_decoders(columns: &[Column]) -> Vec<CellDecoder> {
    columns.iter().map(|c| cell_decoder(c.type_())).collect()
}

fn pg_row_values(row: &Row, decoders: &[CellDecoder]) -> Vec<Option<String>> {
    decoders
        .iter()
        .enumerate()
        .map(|(i, decode)| decode(row, i))
        .collect()
}

fn format_compact_count(n: f64) -> String {
    if n < 0.0 {
        return "0".to_string();
    }
    if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.1}k", n / 1_000.0)
    } else {
        format!("{}", n as i64)
    }
}

fn parse_explain_node(
    node: &serde_json::Value,
    depth: usize,
    total_time: f64,
    out: &mut Vec<ExplainNode>,
    warnings: &mut usize,
) {
    let op = node
        .get("Node Type")
        .and_then(|v| v.as_str())
        .unwrap_or("Node")
        .to_string();
    let rel = node.get("Relation Name").and_then(|v| v.as_str());
    let index_name = node.get("Index Name").and_then(|v| v.as_str());
    let actual_rows = node.get("Actual Rows").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let actual_total_ms = node
        .get("Actual Total Time")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let pct = if total_time > 0.0 {
        ((actual_total_ms / total_time) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    let is_warn = op == "Seq Scan" && actual_rows >= 1000.0;
    if is_warn {
        *warnings += 1;
    }

    let mut detail_parts = Vec::new();
    if let Some(r) = rel {
        detail_parts.push(format!("on {r}"));
    }
    if let Some(idx) = index_name {
        detail_parts.push(format!("using {idx}"));
    }
    if let Some(cond) = node.get("Filter").and_then(|v| v.as_str()) {
        detail_parts.push(format!("Filter: {cond}"));
    }
    if is_warn {
        if let Some(r) = rel {
            detail_parts.push(format!("(Suggest index on {r})"));
        }
    }

    out.push(ExplainNode {
        depth,
        operation: op,
        detail: detail_parts.join(" "),
        actual_rows: format!("{}", actual_rows as i64),
        time_pct: pct,
        time_ms: format!("{actual_total_ms:.2} ms"),
        is_warn,
    });

    if let Some(plans) = node.get("Plans").and_then(|v| v.as_array()) {
        for child in plans {
            parse_explain_node(child, depth + 1, total_time, out, warnings);
        }
    }
}

#[async_trait]
impl DatabaseAdapter for PostgresAdapter {
    fn driver(&self) -> DbDriver {
        DbDriver::Postgres
    }

    fn config(&self) -> &ConnectionConfig {
        &self.config
    }

    async fn test_connection(&self) -> AdapterResult<ConnectionTestInfo> {
        let start = Instant::now();
        let client = self.get_client().await?;
        let row = client
            .query_typed_one("SELECT version()", &[])
            .await
            .map_err(query_error)?;
        let rtt_ms = start.elapsed().as_millis() as u64;
        let full: String = row.get(0);
        let short = full
            .split(" on ")
            .next()
            .unwrap_or(&full)
            .split(',')
            .next()
            .unwrap_or(&full)
            .to_string();
        Ok(ConnectionTestInfo {
            server_version: short,
            rtt_ms,
        })
    }

    async fn list_databases(&self) -> AdapterResult<Vec<String>> {
        let client = self.get_client().await?;
        let rows = client
            .query_typed(
                "SELECT datname
                 FROM pg_database
                 WHERE datistemplate = false
                 ORDER BY datname",
                &[],
            )
            .await
            .map_err(query_error)?;
        let mut dbs: Vec<String> = rows.into_iter().map(|r| r.get(0)).collect();
        if !self.config.database.is_empty() {
            if let Some(pos) = dbs.iter().position(|d| d == &self.config.database) {
                let active = dbs.remove(pos);
                dbs.insert(0, active);
            } else {
                dbs.insert(0, self.config.database.clone());
            }
        }
        if dbs.is_empty() {
            dbs.push(self.config.database.clone());
        }
        Ok(dbs)
    }

    async fn list_schemas(&self) -> AdapterResult<Vec<String>> {
        let client = self.get_client().await?;
        let rows = client
            .query_typed(
                "SELECT schema_name
                 FROM information_schema.schemata
                 WHERE schema_name NOT LIKE 'pg_temp_%'
                   AND schema_name NOT LIKE 'pg_toast%'
                   AND schema_name NOT IN ('pg_catalog', 'information_schema')
                 ORDER BY CASE WHEN schema_name = 'public' THEN 0 ELSE 1 END, schema_name",
                &[],
            )
            .await
            .map_err(query_error)?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    async fn list_schema_objects(&self, schema: &str) -> AdapterResult<SchemaObjects> {
        let schemas = self.list_schemas().await?;
        let client = self.get_client().await?;
        let mut groups: BTreeMap<String, Vec<SchemaObject>> = BTreeMap::new();

        // Pipeline independent catalog queries on the same pooled connection.
        let schema = if schema.is_empty() {
            schemas.first().map(String::as_str).unwrap_or("public")
        } else {
            schema
        };
        let params: &[(&(dyn ToSql + Sync), Type)] = &[(&schema, Type::NAME)];
        let (table_rows, view_rows, func_rows, seq_rows, enum_rows, ext_rows) = tokio::join!(
            client.query_typed(
                "SELECT c.relname,
                        GREATEST(c.reltuples, 0)::float8 AS est_rows,
                        pg_size_pretty(pg_total_relation_size(c.oid)) AS total_size
                 FROM pg_class c
                 JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE n.nspname = $1 AND c.relkind IN ('r', 'p')
                 ORDER BY c.relname",
                params,
            ),
            client.query_typed(
                "SELECT c.relname, c.relkind::text
                 FROM pg_class c
                 JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE n.nspname = $1 AND c.relkind IN ('v', 'm')
                 ORDER BY c.relname",
                params,
            ),
            client.query_typed(
                "SELECT p.proname || '()'
                 FROM pg_proc p
                 JOIN pg_namespace n ON n.oid = p.pronamespace
                 WHERE n.nspname = $1 AND p.prokind = 'f'
                 ORDER BY p.proname
                 LIMIT 60",
                params,
            ),
            client.query_typed(
                "SELECT sequence_name FROM information_schema.sequences WHERE sequence_schema = $1 ORDER BY sequence_name",
                params,
            ),
            client.query_typed(
                "SELECT t.typname
                 FROM pg_type t
                 JOIN pg_namespace n ON n.oid = t.typnamespace
                 WHERE n.nspname = $1 AND t.typtype = 'e'
                 ORDER BY t.typname",
                params,
            ),
            client.query_typed(
                "SELECT extname, 'v' || extversion FROM pg_extension ORDER BY extname",
                &[],
            ),
        );

        // Tables
        let table_rows = table_rows.map_err(query_error)?;
        let tables: Vec<SchemaObject> = table_rows
            .into_iter()
            .map(|r| {
                let name: String = r.get(0);
                let est: f64 = r.get(1);
                let size: String = r.get(2);
                SchemaObject {
                    object_type: SchemaObjectType::Table,
                    name,
                    meta: format!("{} · {}", format_compact_count(est), size.replace(' ', "")),
                    schema: schema.to_string(),
                }
            })
            .collect();
        if !tables.is_empty() {
            groups.insert("数据表".to_string(), tables);
        }

        // Views & Materialized Views
        let view_rows = view_rows.map_err(query_error)?;
        let mut vf_objs: Vec<SchemaObject> = view_rows
            .into_iter()
            .map(|r| {
                let name: String = r.get(0);
                let kind: String = r.get(1);
                let is_mv = kind == "m";
                SchemaObject {
                    object_type: if is_mv {
                        SchemaObjectType::MaterializedView
                    } else {
                        SchemaObjectType::View
                    },
                    name,
                    meta: if is_mv { "物化视图" } else { "视图" }.to_string(),
                    schema: schema.to_string(),
                }
            })
            .collect();

        // Functions
        let func_rows = func_rows.unwrap_or_default();
        for r in func_rows {
            vf_objs.push(SchemaObject {
                object_type: SchemaObjectType::Function,
                name: r.get(0),
                meta: "函数".to_string(),
                schema: schema.to_string(),
            });
        }
        if !vf_objs.is_empty() {
            groups.insert("视图与函数".to_string(), vf_objs);
        }

        // Enums, Sequences & Extensions
        let mut type_objs = Vec::new();
        let seq_rows = seq_rows.unwrap_or_default();
        for r in seq_rows {
            type_objs.push(SchemaObject {
                object_type: SchemaObjectType::Sequence,
                name: r.get(0),
                meta: "序列".to_string(),
                schema: schema.to_string(),
            });
        }

        let enum_rows = enum_rows.unwrap_or_default();
        for r in enum_rows {
            type_objs.push(SchemaObject {
                object_type: SchemaObjectType::Enum,
                name: r.get(0),
                meta: "枚举".to_string(),
                schema: schema.to_string(),
            });
        }

        let ext_rows = ext_rows.unwrap_or_default();
        for r in ext_rows {
            type_objs.push(SchemaObject {
                object_type: SchemaObjectType::Extension,
                name: r.get(0),
                meta: r.get(1),
                schema: schema.to_string(),
            });
        }
        if !type_objs.is_empty() {
            groups.insert("类型与扩展".to_string(), type_objs);
        }

        Ok(SchemaObjects { schemas, groups })
    }

    async fn get_table_structure(&self, schema: &str, table: &str) -> AdapterResult<TableStructure> {
        let client = self.get_client().await?;

        let params: &[(&(dyn ToSql + Sync), Type)] = &[(&schema, Type::NAME), (&table, Type::NAME)];
        // These catalog reads are independent. Send them together on the same connection.
        let (col_rows, idx_rows, con_rows) = tokio::join!(
            client.query_typed(
                "SELECT
                    c.column_name,
                    c.udt_name,
                    c.is_nullable = 'YES' AS nullable,
                    c.column_default,
                    pgd.description,
                    EXISTS (
                        SELECT 1 FROM pg_constraint pk
                        WHERE pk.conrelid = rel.oid AND pk.contype = 'p'
                          AND attr.attnum = ANY(pk.conkey)
                    ) AS is_pk,
                    fk.target_table, fk.target_column,
                    ARRAY(
                        SELECT e.enumlabel::text
                        FROM pg_enum e
                        JOIN pg_type t ON t.oid = e.enumtypid
                        JOIN pg_namespace n ON n.oid = t.typnamespace
                        WHERE t.typname = c.udt_name AND n.nspname = c.udt_schema
                        ORDER BY e.enumsortorder
                    ) AS enum_values
                 FROM information_schema.columns c
                 JOIN pg_namespace ns ON ns.nspname = c.table_schema
                 JOIN pg_class rel ON rel.relnamespace = ns.oid AND rel.relname = c.table_name
                 JOIN pg_attribute attr ON attr.attrelid = rel.oid AND attr.attname = c.column_name
                 LEFT JOIN pg_description pgd ON pgd.objoid = rel.oid
                   AND pgd.classoid = 'pg_class'::regclass AND pgd.objsubid = attr.attnum
                 LEFT JOIN LATERAL (
                     SELECT target.relname AS target_table, ref.attname AS target_column
                     FROM pg_constraint k
                     JOIN pg_class target ON target.oid = k.confrelid
                     JOIN pg_attribute ref ON ref.attrelid = k.confrelid
                       AND ref.attnum = k.confkey[array_position(k.conkey, attr.attnum)]
                     WHERE k.conrelid = rel.oid AND k.contype = 'f' AND attr.attnum = ANY(k.conkey)
                     ORDER BY k.oid LIMIT 1
                 ) fk ON true
                 WHERE c.table_schema = $1 AND c.table_name = $2
                 ORDER BY c.ordinal_position",
                params,
            ),
            client.query_typed(
                "SELECT
                    i.relname AS index_name,
                    array_to_string(array_agg(a.attname ORDER BY x.ordinality), ', ') AS cols,
                    am.amname AS method,
                    ix.indisunique AS is_unique,
                    pg_size_pretty(pg_relation_size(i.oid)) AS idx_size,
                    COALESCE(s.idx_scan, 0)::text AS scans
                 FROM pg_index ix
                 JOIN pg_class t ON t.oid = ix.indrelid
                 JOIN pg_namespace n ON n.oid = t.relnamespace
                 JOIN pg_class i ON i.oid = ix.indexrelid
                 JOIN pg_am am ON am.oid = i.relam
                 LEFT JOIN pg_stat_user_indexes s ON s.indexrelid = i.oid
                 JOIN unnest(ix.indkey) WITH ORDINALITY AS x(attnum, ordinality) ON true
                 JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = x.attnum
                 WHERE n.nspname = $1 AND t.relname = $2
                 GROUP BY i.relname, am.amname, ix.indisunique, i.oid, s.idx_scan
                 ORDER BY i.relname",
                params,
            ),
            client.query_typed(
                "SELECT
                    con.conname,
                    CASE con.contype
                        WHEN 'p' THEN 'PRIMARY KEY'
                        WHEN 'f' THEN 'FOREIGN KEY'
                        WHEN 'u' THEN 'UNIQUE'
                        WHEN 'c' THEN 'CHECK'
                        ELSE 'OTHER'
                    END AS ctype,
                    pg_get_constraintdef(con.oid) AS def
                 FROM pg_constraint con
                 JOIN pg_class c ON c.oid = con.conrelid
                 JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE n.nspname = $1 AND c.relname = $2
                 ORDER BY con.contype, con.conname",
                params,
            ),
        );
        let col_rows = col_rows.map_err(query_error)?;
        let mut columns = Vec::with_capacity(col_rows.len());
        for r in col_rows {
            let udt_name: String = r.get(1);
            columns.push(ColumnDef {
                name: r.get(0),
                data_type: udt_name,
                nullable: r.get(2),
                default_expr: r.get(3),
                comment: r.get(4),
                is_pk: r.get(5),
                fk_table: r.get(6),
                fk_column: r.get(7),
                enum_values: r.get(8),
            });
        }

        // Indexes
        let indexes: Vec<IndexDef> = idx_rows
            .unwrap_or_default()
            .into_iter()
            .map(|r| IndexDef {
                name: r.get(0),
                columns: r.get(1),
                method: r.get(2),
                is_unique: r.get(3),
                size: r.get(4),
                scans: r.get(5),
            })
            .collect();

        // Constraints
        let constraints: Vec<ConstraintDef> = con_rows
            .unwrap_or_default()
            .into_iter()
            .map(|r| ConstraintDef {
                name: r.get(0),
                constraint_type: r.get(1),
                definition: r.get(2),
            })
            .collect();

        // Generate DDL
        let col_lines: Vec<String> = columns
            .iter()
            .map(|c| {
                let mut line = format!("  {} {}", quote_ident(&c.name), c.data_type);
                if !c.nullable {
                    line.push_str(" NOT NULL");
                }
                if let Some(def) = &c.default_expr {
                    line.push_str(&format!(" DEFAULT {def}"));
                }
                line
            })
            .collect();
        let mut ddl = format!(
            "CREATE TABLE {}.{} (\n{}\n);",
            quote_ident(schema),
            quote_ident(table),
            col_lines.join(",\n")
        );
        if !indexes.is_empty() {
            ddl.push_str("\n\n");
            let idx_lines: Vec<String> = indexes
                .iter()
                .map(|x| {
                    format!(
                        "CREATE {}INDEX {} ON {}.{} USING {} ({});",
                        if x.is_unique { "UNIQUE " } else { "" },
                        quote_ident(&x.name),
                        quote_ident(schema),
                        quote_ident(table),
                        x.method,
                        x.columns
                    )
                })
                .collect();
            ddl.push_str(&idx_lines.join("\n"));
        }

        Ok(TableStructure {
            schema: schema.to_string(),
            table: table.to_string(),
            columns: columns.into(),
            indexes,
            constraints,
            ddl,
        })
    }

    async fn query_table_page(
        &self,
        query: &TableDataQuery,
        columns: &[ColumnDef],
    ) -> AdapterResult<TablePage> {
        let start = Instant::now();
        let client = self.get_client().await?;
        let fq = format!("{}.{}", quote_ident(&query.schema), quote_ident(&query.table));

        let projection = if columns.is_empty() {
            "*".to_owned()
        } else {
            columns.iter().map(|c| quote_ident(&c.name)).collect::<Vec<_>>().join(", ")
        };
        let mut sql = format!("SELECT {projection} FROM {fq}");
        if let Some(w) = &query.where_clause {
            let trimmed = w.trim();
            if !trimmed.is_empty() {
                let upper = trimmed.to_uppercase();
                let is_sql_expr = trimmed.contains('=')
                    || trimmed.contains('>')
                    || trimmed.contains('<')
                    || upper.contains(" LIKE ")
                    || upper.contains(" ILIKE ")
                    || upper.contains(" IS ")
                    || upper.contains(" IN ")
                    || upper.contains(" AND ")
                    || upper.contains(" OR ");
                if is_sql_expr {
                    sql.push_str(&format!(" WHERE {trimmed}"));
                } else {
                    let esc = trimmed.replace('\'', "''");
                    let preds: Vec<String> = columns
                        .iter()
                        .map(|c| format!("{}::text ILIKE '%{esc}%'", quote_ident(&c.name)))
                        .collect();
                    if !preds.is_empty() {
                        sql.push_str(&format!(" WHERE ({})", preds.join(" OR ")));
                    }
                }
            }
        }

        if let Some(sort_col) = &query.sort_col {
            sql.push_str(&format!(
                " ORDER BY {} {}",
                quote_ident(sort_col),
                if query.sort_asc { "ASC" } else { "DESC" }
            ));
        }

        let params: &[(&(dyn ToSql + Sync), Type)] = &[
            (&query.schema, Type::NAME),
            (&query.table, Type::NAME),
        ];
        // Consume the row stream while the estimate is in flight on the same connection.
        let read_page = async {
            let stream = Self::page_stream(&client, sql, query).await?;
            let mut stream = std::pin::pin!(stream);
            let mut rows = Vec::new();
            let mut decoders = None;
            let mut has_more = false;
            while let Some(row) = stream.try_next().await.map_err(query_error)? {
                if rows.len() < query.limit {
                    let decoders = decoders.get_or_insert_with(|| column_decoders(row.columns()));
                    rows.push(pg_row_values(&row, decoders));
                } else {
                    has_more = true;
                }
            }
            Ok::<_, AdapterError>((rows, has_more))
        };
        let (page, estimate) = tokio::join!(
            read_page,
            client.query_typed_opt(
                "SELECT GREATEST(c.reltuples, 0)::bigint
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE n.nspname = $1 AND c.relname = $2",
                params,
            ),
        );
        let (rows, has_more) = page?;
        let total_estimate = estimate.ok().flatten()
            .map(|r| r.get::<_, i64>(0)).unwrap_or(rows.len() as i64);

        Ok(TablePage {
            rows,
            has_more,
            total_estimate,
            elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
        })
    }

    async fn execute_sql(&self, sql: &str, read_only_override: bool) -> AdapterResult<QueryResult> {
        if (self.config.read_only || read_only_override) && !is_read_only_sql(sql) {
            return Err(AdapterError::ReadOnlyViolation);
        }

        let start = Instant::now();
        let client = self.get_client().await?;
        let trimmed = sql.trim();

        if is_read_only_sql(trimmed) {
            let statement = client.prepare(trimmed).await.map_err(query_error)?;
            let columns = statement.columns().iter().map(|c| c.name().to_owned()).collect();
            let column_types = statement
                .columns()
                .iter()
                .map(|c| c.type_().name().to_owned())
                .collect();
            let decoders = column_decoders(statement.columns());
            let rows: Vec<_> = client
                .query_raw(&statement, std::iter::empty::<&(dyn ToSql + Sync)>())
                .await
                .map_err(query_error)?
                .map_ok(|row| pg_row_values(&row, &decoders))
                .try_collect()
                .await
                .map_err(query_error)?;
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            let count = rows.len() as u64;
            Ok(QueryResult {
                columns,
                column_types,
                rows,
                affected_rows: count,
                elapsed_ms,
                message: format!("✓ Query returned {count} rows in {elapsed_ms:.1} ms."),
            })
        } else {
            let affected = client
                .execute(trimmed, &[])
                .await
                .map_err(query_error)?;
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            Ok(QueryResult {
                columns: Vec::new(),
                column_types: Vec::new(),
                rows: Vec::new(),
                affected_rows: affected,
                elapsed_ms,
                message: format!("✓ Statement executed ({affected} rows affected) in {elapsed_ms:.1} ms."),
            })
        }
    }

    async fn explain_sql(&self, sql: &str) -> AdapterResult<ExplainPlan> {
        let mut client = self.get_client().await?;
        let clean = sql.trim().trim_end_matches(';');
        let explain_stmt = if clean.to_uppercase().starts_with("EXPLAIN") {
            clean.to_string()
        } else {
            format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {clean}")
        };

        let tx = client
            .transaction()
            .await
            .map_err(query_error)?;
        let row = tx
            .query_one(&explain_stmt, &[])
            .await
            .map_err(query_error)?;
        let _ = tx.rollback().await;

        let json_val: serde_json::Value = row.get(0);
        let raw_json = serde_json::to_string_pretty(&json_val).unwrap_or_default();
        let top = json_val
            .as_array()
            .and_then(|a| a.first())
            .ok_or_else(|| AdapterError::Query("Unexpected EXPLAIN JSON format".into()))?;

        let total_time_ms = top
            .get("Execution Time")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let planning_time_ms = top
            .get("Planning Time")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let shared_hit_blocks = top
            .get("Plan")
            .and_then(|p| p.get("Shared Hit Blocks"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let mut nodes = Vec::new();
        let mut warnings = 0;
        if let Some(plan) = top.get("Plan") {
            parse_explain_node(plan, 0, total_time_ms.max(0.01), &mut nodes, &mut warnings);
        }

        Ok(ExplainPlan {
            total_time_ms,
            planning_time_ms,
            shared_hit_blocks,
            warnings,
            nodes,
            raw_json,
        })
    }

    async fn commit_staged_changes(&self, changes: &[StagedChange]) -> AdapterResult<u64> {
        if self.config.read_only {
            return Err(AdapterError::ReadOnlyViolation);
        }
        if changes.is_empty() {
            return Ok(0);
        }
        let mut client = self.get_client().await?;
        let tx = client
            .transaction()
            .await
            .map_err(query_error)?;

        let sql_script = generate_staged_sql(changes);
        let mut affected = 0u64;
        for line in sql_script.lines() {
            let l = line.trim();
            if l == "BEGIN;" || l == "COMMIT;" || l.is_empty() {
                continue;
            }
            let n = tx
                .execute(l, &[])
                .await
                .map_err(|e| AdapterError::Query(format!("Commit failed on `{l}`: {e}")))?;
            affected += n;
        }
        tx.commit()
            .await
            .map_err(query_error)?;
        Ok(affected)
    }

    async fn get_server_metrics(&self) -> AdapterResult<ServerMetrics> {
        let client = self.get_client().await?;

        let conn_row = client
            .query_one(
                "SELECT
                    COUNT(*)::bigint AS total,
                    COUNT(*) FILTER (WHERE state = 'active')::bigint AS active,
                    COUNT(*) FILTER (WHERE state = 'idle')::bigint AS idle,
                    COUNT(*) FILTER (WHERE wait_event IS NOT NULL AND state = 'active')::bigint AS waiting,
                    (SELECT setting::bigint FROM pg_settings WHERE name = 'max_connections') AS max_conn
                 FROM pg_stat_activity",
                &[],
            )
            .await
            .map_err(query_error)?;

        let db_row = client
            .query_one(
                "SELECT
                    CASE WHEN (blks_hit + blks_read) = 0 THEN 100.0
                         ELSE ROUND((blks_hit::numeric / (blks_hit + blks_read)) * 100.0, 2)::float8
                    END AS hit_ratio,
                    pg_size_pretty(pg_database_size(current_database())) AS db_size,
                    (xact_commit + xact_rollback)::bigint AS xacts
                 FROM pg_stat_database
                 WHERE datname = current_database()",
                &[],
            )
            .await
            .map_err(query_error)?;

        let sess_rows = client
            .query(
                "SELECT
                    pid,
                    COALESCE(usename, 'system') AS usename,
                    COALESCE(state, 'unknown') AS state,
                    COALESCE(ROUND(EXTRACT(EPOCH FROM (now() - query_start))::numeric, 1)::text || 's', '0.0s') AS dur,
                    COALESCE(LEFT(query, 240), '') AS query
                 FROM pg_stat_activity
                 WHERE pid <> pg_backend_pid()
                 ORDER BY query_start DESC NULLS LAST
                 LIMIT 30",
                &[],
            )
            .await
            .unwrap_or_default();

        let sessions = sess_rows
            .into_iter()
            .map(|r| SessionInfo {
                pid: r.get(0),
                username: r.get(1),
                state: r.get(2),
                duration: r.get(3),
                query: r.get(4),
            })
            .collect();

        let storage_rows = client
            .query(
                "SELECT
                    schemaname,
                    relname,
                    pg_size_pretty(pg_total_relation_size(relid)) AS total_size,
                    pg_size_pretty(pg_relation_size(relid)) AS data_size,
                    pg_size_pretty(pg_indexes_size(relid)) AS idx_size
                 FROM pg_stat_user_tables
                 ORDER BY pg_total_relation_size(relid) DESC
                 LIMIT 10",
                &[],
            )
            .await
            .unwrap_or_default();

        let top_tables = storage_rows
            .into_iter()
            .map(|r| TableStorageInfo {
                schema: r.get(0),
                name: r.get(1),
                total_size: r.get(2),
                data_size: r.get(3),
                index_size: r.get(4),
            })
            .collect();

        Ok(ServerMetrics {
            active_connections: conn_row.get(0),
            active_count: conn_row.get(1),
            idle_count: conn_row.get(2),
            waiting_count: conn_row.get(3),
            max_connections: conn_row.get(4),
            cache_hit_ratio: db_row.get(0),
            db_size: db_row.get(1),
            data_size: "-".to_string(),
            index_size: "-".to_string(),
            tps: db_row.get::<_, i64>(2).min(99999),
            avg_latency_ms: 4.2,
            sessions,
            top_tables,
        })
    }

    async fn cancel_session(&self, pid: i32, terminate: bool) -> AdapterResult<bool> {
        let client = self.get_client().await?;
        let fn_name = if terminate {
            "pg_terminate_backend"
        } else {
            "pg_cancel_backend"
        };
        let row = client
            .query_one(&format!("SELECT {fn_name}($1)"), &[&pid])
            .await
            .map_err(query_error)?;
        Ok(row.get(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires VELODB_TEST_POSTGRES_URI with temporary table privileges"]
    async fn large_enum_page_and_changed_prepared_result() {
        let uri = std::env::var("VELODB_TEST_POSTGRES_URI").unwrap();
        let adapter = PostgresAdapter::new(ConnectionConfig::from_uri(&uri).unwrap()).unwrap();
        let client = adapter.get_client().await.unwrap();
        client
            .batch_execute(
                "CREATE TEMP TABLE velo_source AS SELECT i FROM generate_series(1, 5001) i;
             CREATE TYPE pg_temp.velo_label AS ENUM ('active');
             CREATE TEMP VIEW velo_page AS SELECT i, 'active'::pg_temp.velo_label AS label,
               repeat('x', 1024) AS payload FROM pg_temp.velo_source;",
            )
            .await
            .unwrap();
        drop(client);

        let q = TableDataQuery {
            schema: "pg_temp".into(),
            table: "velo_page".into(),
            where_clause: None,
            sort_col: None,
            sort_asc: true,
            limit: 5000,
            offset: 0,
        };
        let page = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            adapter.query_table_page(&q, &[]),
        )
        .await
        .expect("a large page must drain even when enum metadata is cold")
        .unwrap();
        assert_eq!(page.rows.len(), 5000);
        assert!(page.has_more);
        assert_eq!(page.rows[4999][1].as_deref(), Some("active"));
        drop(page);

        let client = adapter.get_client().await.unwrap();
        client
            .batch_execute(
                "DROP VIEW pg_temp.velo_page;
             CREATE TEMP VIEW velo_page AS SELECT 42::oid AS id;",
            )
            .await
            .unwrap();
        drop(client);
        let page = adapter.query_table_page(&q, &[]).await.unwrap();
        assert_eq!(page.rows, [vec![Some("42".to_owned())]]);
        assert!(!page.has_more);

        let client = adapter.get_client().await.unwrap();
        client.batch_execute("DEALLOCATE ALL").await.unwrap();
        drop(client);
        let page = adapter.query_table_page(&q, &[]).await.unwrap();
        assert_eq!(page.rows, [vec![Some("42".to_owned())]]);

        let mut filtered = q;
        filtered.where_clause = Some("id::text = '$1'".into());
        assert!(adapter.query_table_page(&filtered, &[]).await.unwrap().rows.is_empty());
        filtered.where_clause = Some("id = $1".into());
        assert!(adapter.query_table_page(&filtered, &[]).await.is_err());

        let empty = adapter
            .execute_sql("SELECT 1 AS id WHERE false", true)
            .await
            .unwrap();
        assert!(empty.rows.is_empty());
        assert_eq!(empty.columns, ["id"]);
        assert_eq!(empty.column_types, ["int4"]);
    }

    #[test]
    fn test_uri_parsing_and_adapter_factory() {
        let cfg = ConnectionConfig::from_postgres_uri(
            "postgresql://alice:secret@db.internal:5433/velodb?sslmode=require",
        )
        .unwrap();
        assert_eq!(cfg.driver, DbDriver::Postgres);
        assert_eq!(cfg.host, "db.internal");
        assert_eq!(cfg.port, 5433);
        assert_eq!(cfg.database, "velodb");
        assert_eq!(cfg.username, "alice");
        assert_eq!(cfg.ssl_mode, SslMode::Require);
        assert!(create_adapter(cfg).is_ok());

        let mut sqlite_cfg = ConnectionConfig::from_postgres_uri("postgresql://localhost/db").unwrap();
        sqlite_cfg.driver = DbDriver::Sqlite;
        assert!(matches!(
            create_adapter(sqlite_cfg),
            Err(AdapterError::UnsupportedDriver(_))
        ));
    }

    #[test]
    fn test_staged_sql_generation() {
        let changes = vec![
            StagedChange {
                schema: "public".into(),
                table: "users".into(),
                row_idx: 0,
                pk_col: "id".into(),
                pk_val: Some("42".into()),
                kind: StagedChangeKind::Update {
                    col: "plan".into(),
                    old_val: Some("free".into()),
                    new_val: Some("enterprise".into()),
                },
            },
            StagedChange {
                schema: "public".into(),
                table: "users".into(),
                row_idx: 1,
                pk_col: "id".into(),
                pk_val: Some("99".into()),
                kind: StagedChangeKind::Delete,
            },
        ];
        let sql = generate_staged_sql(&changes);
        assert!(sql.starts_with("BEGIN;"));
        assert!(sql.contains("UPDATE \"public\".\"users\" SET \"plan\" = 'enterprise' WHERE \"id\" = '42';"));
        assert!(sql.contains("DELETE FROM \"public\".\"users\" WHERE \"id\" = '99';"));
        assert!(sql.ends_with("COMMIT;"));
    }

    #[test]
    fn test_export_formats_and_readonly_guard() {
        let cols = vec!["id".into(), "email".into()];
        let rows = vec![vec![Some("1".into()), Some("a@b.com".into())]];
        let csv = export_rows(&cols, &rows, "public", "users", ExportFormat::Csv);
        assert_eq!(csv, "id,email\n1,a@b.com");

        let sql = export_rows(&cols, &rows, "public", "users", ExportFormat::SqlInsert);
        assert_eq!(
            sql,
            "INSERT INTO \"public\".\"users\" (\"id\", \"email\") VALUES ('1', 'a@b.com');"
        );

        assert!(is_read_only_sql("SELECT * FROM public.users"));
        assert!(is_read_only_sql("EXPLAIN ANALYZE SELECT 1"));
        assert!(!is_read_only_sql("DELETE FROM public.users"));
    }
}
