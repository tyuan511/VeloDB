use super::*;
use mysql_async::prelude::*;
use mysql_async::{OptsBuilder, Pool, Row, SslOpts, TxOpts, Value};
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

pub struct MySqlAdapter {
    config: ConnectionConfig,
    pool: Pool,
}

impl MySqlAdapter {
    pub fn new(config: ConnectionConfig) -> AdapterResult<Self> {
        let mut builder = OptsBuilder::default()
            // Pool resets restore server defaults. Use the same utf8mb4 version
            // threshold as the driver's handshake when restoring the client encoding.
            .setup(vec!["/*!50503 SET NAMES utf8mb4 */"])
            .ip_or_hostname(config.host.clone())
            .tcp_port(config.port)
            .user(if config.username.is_empty() {
                None
            } else {
                Some(config.username.clone())
            })
            .pass(if config.password.is_empty() {
                None
            } else {
                Some(config.password.clone())
            })
            .db_name(if config.database.is_empty() {
                None
            } else {
                Some(config.database.clone())
            });

        builder = match config.ssl_mode {
            SslMode::Disable => builder.ssl_opts(None::<SslOpts>),
            SslMode::Prefer | SslMode::Require => builder.ssl_opts(Some(
                SslOpts::default()
                    .with_danger_accept_invalid_certs(true)
                    .with_danger_skip_domain_validation(true),
            )),
            SslMode::VerifyFull => builder.ssl_opts(Some(SslOpts::default())),
        };

        let pool = Pool::new(builder);
        Ok(Self { config, pool })
    }

    async fn read_schemas(&self, conn: &mut mysql_async::Conn) -> AdapterResult<Vec<String>> {
        let rows: Vec<Row> = conn
            .query(
                "SELECT SCHEMA_NAME \
                 FROM information_schema.SCHEMATA \
                 WHERE SCHEMA_NAME NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys') \
                 ORDER BY SCHEMA_NAME",
            )
            .await
            .map_err(|e| AdapterError::Query(e.to_string()))?;

        let mut schemas: Vec<String> = rows.iter().filter_map(|r| row_str(r, 0)).collect();
        if !self.config.database.is_empty() {
            if let Some(pos) = schemas.iter().position(|s| s == &self.config.database) {
                let active = schemas.remove(pos);
                schemas.insert(0, active);
            } else {
                schemas.insert(0, self.config.database.clone());
            }
        }
        Ok(schemas)
    }

    async fn get_conn(&self) -> AdapterResult<mysql_async::Conn> {
        self.pool
            .get_conn()
            .await
            .map_err(|e| AdapterError::Connection(e.to_string()))
    }
}

pub fn quote_ident(ident: &str) -> String {
    format!("`{}`", ident.replace('`', "``"))
}

pub fn sql_literal(val: &Option<String>) -> String {
    match val {
        None => "NULL".to_string(),
        Some(s) => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "''")),
    }
}

pub fn generate_mysql_staged_sql(changes: &[StagedChange]) -> String {
    let mut lines = vec!["START TRANSACTION;".to_string()];
    for ch in changes {
        let fq = format!("{}.{}", quote_ident(&ch.schema), quote_ident(&ch.table));
        match &ch.kind {
            StagedChangeKind::Update { col, new_val, .. } => {
                let pk_cond = match &ch.pk_val {
                    Some(v) => format!(
                        "{} = {}",
                        quote_ident(&ch.pk_col),
                        sql_literal(&Some(v.clone()))
                    ),
                    None => "FALSE /* missing PK */".to_string(),
                };
                lines.push(format!(
                    "UPDATE {fq} SET {} = {} WHERE {pk_cond};",
                    quote_ident(col),
                    sql_literal(new_val)
                ));
            }
            StagedChangeKind::Insert { values } => {
                if values.is_empty() {
                    lines.push(format!("INSERT INTO {fq} () VALUES ();"));
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
                    Some(v) => format!(
                        "{} = {}",
                        quote_ident(&ch.pk_col),
                        sql_literal(&Some(v.clone()))
                    ),
                    None => "FALSE".to_string(),
                };
                lines.push(format!("DELETE FROM {fq} WHERE {pk_cond};"));
            }
        }
    }
    lines.push("COMMIT;".to_string());
    lines.join("\n")
}

pub fn is_read_only_sql(sql: &str) -> bool {
    let trimmed = sql.trim_start();
    let upper = trimmed
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with("--") && !t.starts_with('#')
        })
        .collect::<Vec<_>>()
        .join(" ")
        .trim_start()
        .to_uppercase();
    upper.starts_with("SELECT")
        || upper.starts_with("WITH")
        || upper.starts_with("EXPLAIN")
        || upper.starts_with("SHOW")
        || upper.starts_with("DESCRIBE")
        || upper.starts_with("DESC ")
        || upper.starts_with("VALUES")
}

fn mysql_value_to_string(val: Value) -> Option<String> {
    match val {
        Value::NULL => None,
        Value::Bytes(bytes) => match String::from_utf8(bytes) {
            Ok(s) => Some(s),
            Err(e) => {
                let raw = e.into_bytes();
                let preview: String = raw
                    .iter()
                    .take(32)
                    .map(|b| format!("{b:02x}"))
                    .collect();
                Some(format!("0x{preview} ({}B)", raw.len()))
            }
        },
        Value::Int(i) => Some(i.to_string()),
        Value::UInt(u) => Some(u.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Double(d) => Some(d.to_string()),
        Value::Date(y, m, d, 0, 0, 0, 0) => Some(format!("{y:04}-{m:02}-{d:02}")),
        Value::Date(y, m, d, hh, mm, ss, 0) => {
            Some(format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}"))
        }
        Value::Date(y, m, d, hh, mm, ss, us) => {
            Some(format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}.{us:06}"))
        }
        Value::Time(neg, days, hh, mm, ss, us) => {
            let total_hours = days as u64 * 24 + hh as u64;
            let sign = if neg { "-" } else { "" };
            if us == 0 {
                Some(format!("{sign}{total_hours:02}:{mm:02}:{ss:02}"))
            } else {
                Some(format!("{sign}{total_hours:02}:{mm:02}:{ss:02}.{us:06}"))
            }
        }
    }
}

fn row_str(row: &Row, idx: usize) -> Option<String> {
    row.as_ref(idx).cloned().and_then(mysql_value_to_string)
}

fn into_cells(row: Row) -> Vec<Option<String>> {
    row.unwrap().into_iter().map(mysql_value_to_string).collect()
}

fn row_i64(row: &Row, idx: usize) -> i64 {
    match row.as_ref(idx) {
        Some(Value::Int(v)) => *v,
        Some(Value::UInt(v)) => *v as i64,
        Some(Value::Bytes(b)) => std::str::from_utf8(b)
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0),
        Some(Value::Double(v)) => *v as i64,
        Some(Value::Float(v)) => *v as i64,
        _ => 0,
    }
}

fn format_bytes(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GB", b / (1024.0 * 1024.0 * 1024.0))
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    } else if b >= 1024.0 {
        format!("{:.1} KB", b / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn parse_mysql_enum_values(col_type: &str) -> Vec<String> {
    let trimmed = col_type.trim();
    let lower = trimmed.to_lowercase();
    if !lower.starts_with("enum(") || !trimmed.ends_with(')') {
        return Vec::new();
    }
    let inner = &trimmed[5..trimmed.len() - 1];
    let mut vals = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quote {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    current.push('\'');
                } else {
                    in_quote = false;
                    vals.push(std::mem::take(&mut current));
                }
            } else if c == '\\' {
                if let Some(next_c) = chars.next() {
                    current.push(next_c);
                }
            } else {
                current.push(c);
            }
        } else if c == '\'' {
            in_quote = true;
        }
    }
    vals
}

fn parse_mysql_explain_json(raw: &str) -> ExplainPlan {
    let mut nodes = Vec::new();
    let mut warnings = 0usize;

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) {
        if let Some(qb) = val.get("query_block") {
            fn walk(
                v: &serde_json::Value,
                depth: usize,
                nodes: &mut Vec<ExplainNode>,
                warnings: &mut usize,
            ) {
                if let Some(tbl) = v.get("table") {
                    let table_name = tbl
                        .get("table_name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("table");
                    let access_type = tbl
                        .get("access_type")
                        .and_then(|x| x.as_str())
                        .unwrap_or("scan");
                    let key = tbl.get("key").and_then(|x| x.as_str());
                    let rows = tbl
                        .get("rows_examined_per_scan")
                        .and_then(|x| x.as_i64())
                        .unwrap_or(0);
                    let cost = tbl
                        .get("cost_info")
                        .and_then(|c| c.get("prefix_cost"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("0");
                    let is_warn = access_type.eq_ignore_ascii_case("ALL") && rows > 100;
                    if is_warn {
                        *warnings += 1;
                    }
                    let detail = match key {
                        Some(k) => format!("on {table_name} using {k}"),
                        None => format!("on {table_name}"),
                    };
                    nodes.push(ExplainNode {
                        depth,
                        operation: format!("Table Scan ({access_type})"),
                        detail,
                        actual_rows: format!("{rows} rows"),
                        time_pct: 100.0,
                        time_ms: format!("cost {cost}"),
                        is_warn,
                    });
                }
                if let Some(nested) = v.get("nested_loop").and_then(|x| x.as_array()) {
                    for item in nested {
                        walk(item, depth + 1, nodes, warnings);
                    }
                }
                if let Some(ord) = v.get("ordering_operation") {
                    nodes.push(ExplainNode {
                        depth,
                        operation: "Ordering (Filesort)".to_string(),
                        detail: if ord
                            .get("using_filesort")
                            .and_then(|x| x.as_bool())
                            .unwrap_or(false)
                        {
                            "using_filesort = true".to_string()
                        } else {
                            "indexed order".to_string()
                        },
                        actual_rows: "-".to_string(),
                        time_pct: 0.0,
                        time_ms: "-".to_string(),
                        is_warn: false,
                    });
                    walk(ord, depth + 1, nodes, warnings);
                }
                if let Some(grp) = v.get("grouping_operation") {
                    nodes.push(ExplainNode {
                        depth,
                        operation: "Grouping".to_string(),
                        detail: "GROUP BY".to_string(),
                        actual_rows: "-".to_string(),
                        time_pct: 0.0,
                        time_ms: "-".to_string(),
                        is_warn: false,
                    });
                    walk(grp, depth + 1, nodes, warnings);
                }
            }
            walk(qb, 0, &mut nodes, &mut warnings);
        }
    }

    if nodes.is_empty() {
        nodes.push(ExplainNode {
            depth: 0,
            operation: "MySQL Query Plan".to_string(),
            detail: "See raw EXPLAIN JSON".to_string(),
            actual_rows: "-".to_string(),
            time_pct: 100.0,
            time_ms: "-".to_string(),
            is_warn: false,
        });
    }

    ExplainPlan {
        total_time_ms: 0.0,
        planning_time_ms: 0.0,
        shared_hit_blocks: 0,
        warnings,
        nodes,
        raw_json: raw.to_string(),
    }
}

#[async_trait]
impl DatabaseAdapter for MySqlAdapter {
    fn driver(&self) -> DbDriver {
        DbDriver::Mysql
    }

    fn config(&self) -> &ConnectionConfig {
        &self.config
    }

    async fn test_connection(&self) -> AdapterResult<ConnectionTestInfo> {
        let started = Instant::now();
        let mut conn = self.get_conn().await?;
        let ver: Option<String> = conn
            .query_first("SELECT VERSION()")
            .await
            .map_err(|e| AdapterError::Query(e.to_string()))?;
        let rtt_ms = started.elapsed().as_millis() as u64;
        let server_version = match ver {
            Some(v) => format!("MySQL {v}"),
            None => "MySQL".to_string(),
        };
        Ok(ConnectionTestInfo {
            server_version,
            rtt_ms,
        })
    }

    async fn list_databases(&self) -> AdapterResult<Vec<String>> {
        self.list_schemas().await
    }

    async fn list_schemas(&self) -> AdapterResult<Vec<String>> {
        let mut conn = self.get_conn().await?;
        self.read_schemas(&mut conn).await
    }

    async fn list_schema_objects(&self, schema: &str) -> AdapterResult<SchemaObjects> {
        let mut conn = self.get_conn().await?;
        let schemas = self.read_schemas(&mut conn).await?;
        let target_schema = if schema.is_empty() {
            String::new()
        } else if schema == "public" && !schemas.iter().any(|s| s == "public") {
            schemas.first().cloned().unwrap_or_default()
        } else {
            schema.to_string()
        };
        if target_schema.is_empty() {
            return Ok(SchemaObjects {
                schemas,
                groups: BTreeMap::new(),
            });
        }

        let mut groups: BTreeMap<String, Vec<SchemaObject>> = BTreeMap::new();

        // Tables & Views
        let table_rows: Vec<Row> = conn
            .exec(
                "SELECT TABLE_NAME, TABLE_TYPE, COALESCE(TABLE_ROWS, 0), \
                        COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0) \
                 FROM information_schema.TABLES \
                 WHERE TABLE_SCHEMA = ? \
                 ORDER BY TABLE_NAME",
                (&target_schema,),
            )
            .await
            .map_err(|e| AdapterError::Query(e.to_string()))?;

        let mut tables = Vec::new();
        let mut views = Vec::new();
        for r in &table_rows {
            let name = row_str(r, 0).unwrap_or_default();
            let ttype = row_str(r, 1).unwrap_or_default();
            let est_rows = row_i64(r, 2);
            let total_bytes = row_i64(r, 3);
            if ttype.eq_ignore_ascii_case("VIEW") {
                views.push(SchemaObject {
                    object_type: SchemaObjectType::View,
                    name,
                    meta: "VIEW".to_string(),
                    schema: target_schema.clone(),
                });
            } else {
                tables.push(SchemaObject {
                    object_type: SchemaObjectType::Table,
                    name,
                    meta: format!("~{} 行 · {}", est_rows, format_bytes(total_bytes)),
                    schema: target_schema.clone(),
                });
            }
        }
        if !tables.is_empty() {
            groups.insert("数据表".to_string(), tables);
        }
        if !views.is_empty() {
            groups.insert("视图".to_string(), views);
        }

        // Functions & Procedures
        let routine_rows: Vec<Row> = conn
            .exec(
                "SELECT ROUTINE_NAME, ROUTINE_TYPE \
                 FROM information_schema.ROUTINES \
                 WHERE ROUTINE_SCHEMA = ? \
                 ORDER BY ROUTINE_NAME",
                (&target_schema,),
            )
            .await
            .unwrap_or_default();
        if !routine_rows.is_empty() {
            let funcs = routine_rows
                .iter()
                .map(|r| SchemaObject {
                    object_type: SchemaObjectType::Function,
                    name: row_str(r, 0).unwrap_or_default(),
                    meta: row_str(r, 1).unwrap_or_else(|| "ROUTINE".to_string()),
                    schema: target_schema.clone(),
                })
                .collect();
            groups.insert("函数与过程".to_string(), funcs);
        }

        Ok(SchemaObjects { schemas, groups })
    }

    async fn get_table_structure(&self, schema: &str, table: &str) -> AdapterResult<TableStructure> {
        let mut conn = self.get_conn().await?;

        // Foreign keys
        let fk_rows: Vec<Row> = conn
            .exec(
                "SELECT COLUMN_NAME, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME \
                 FROM information_schema.KEY_COLUMN_USAGE \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND REFERENCED_TABLE_NAME IS NOT NULL",
                (schema, table),
            )
            .await
            .unwrap_or_default();
        let mut fk_map: HashMap<String, (String, String)> = HashMap::new();
        for r in &fk_rows {
            if let (Some(col), Some(rt), Some(rc)) = (row_str(r, 0), row_str(r, 1), row_str(r, 2)) {
                fk_map.insert(col, (rt, rc));
            }
        }

        // Columns
        let col_rows: Vec<Row> = conn
            .exec(
                "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, COLUMN_KEY, COLUMN_COMMENT \
                 FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                 ORDER BY ORDINAL_POSITION",
                (schema, table),
            )
            .await
            .map_err(|e| AdapterError::Query(e.to_string()))?;

        let columns: Vec<ColumnDef> = col_rows
            .iter()
            .map(|r| {
                let name = row_str(r, 0).unwrap_or_default();
                let data_type = row_str(r, 1).unwrap_or_default();
                let nullable = row_str(r, 2).as_deref() == Some("YES");
                let default_expr = row_str(r, 3);
                let is_pk = row_str(r, 4).as_deref() == Some("PRI");
                let comment = row_str(r, 5).filter(|s| !s.is_empty());
                let enum_values = parse_mysql_enum_values(&data_type);
                let (fk_table, fk_column) = fk_map
                    .get(&name)
                    .map(|(t, c)| (Some(t.clone()), Some(c.clone())))
                    .unwrap_or((None, None));
                ColumnDef {
                    name,
                    data_type,
                    is_pk,
                    fk_table,
                    fk_column,
                    nullable,
                    default_expr,
                    comment,
                    enum_values,
                }
            })
            .collect();

        // Indexes
        let idx_rows: Vec<Row> = conn
            .exec(
                "SELECT INDEX_NAME, NON_UNIQUE, INDEX_TYPE, \
                        GROUP_CONCAT(COLUMN_NAME ORDER BY SEQ_IN_INDEX SEPARATOR ', ') \
                 FROM information_schema.STATISTICS \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                 GROUP BY INDEX_NAME, NON_UNIQUE, INDEX_TYPE \
                 ORDER BY INDEX_NAME",
                (schema, table),
            )
            .await
            .unwrap_or_default();
        let indexes: Vec<IndexDef> = idx_rows
            .iter()
            .map(|r| IndexDef {
                name: row_str(r, 0).unwrap_or_default(),
                is_unique: row_i64(r, 1) == 0,
                method: row_str(r, 2).unwrap_or_else(|| "BTREE".to_string()),
                columns: row_str(r, 3).unwrap_or_default(),
                size: "-".to_string(),
                scans: "-".to_string(),
            })
            .collect();

        // Constraints
        let con_rows: Vec<Row> = conn
            .exec(
                "SELECT CONSTRAINT_NAME, CONSTRAINT_TYPE \
                 FROM information_schema.TABLE_CONSTRAINTS \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                 ORDER BY CONSTRAINT_NAME",
                (schema, table),
            )
            .await
            .unwrap_or_default();
        let constraints: Vec<ConstraintDef> = con_rows
            .iter()
            .map(|r| {
                let cname = row_str(r, 0).unwrap_or_default();
                let ctype = row_str(r, 1).unwrap_or_default();
                ConstraintDef {
                    name: cname.clone(),
                    constraint_type: ctype.clone(),
                    definition: format!("{ctype} ({cname})"),
                }
            })
            .collect();

        // DDL via SHOW CREATE TABLE
        let fq = format!("{}.{}", quote_ident(schema), quote_ident(table));
        let ddl = match conn.query_first::<Row, _>(format!("SHOW CREATE TABLE {fq}")).await {
            Ok(Some(r)) => row_str(&r, 1).unwrap_or_default(),
            _ => format!("-- Unable to fetch DDL for {fq}"),
        };

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
        let started = Instant::now();
        let mut conn = self.get_conn().await?;

        let fq = format!(
            "{}.{}",
            quote_ident(&query.schema),
            quote_ident(&query.table)
        );
        let col_list = if columns.is_empty() {
            "*".to_string()
        } else {
            columns
                .iter()
                .map(|c| quote_ident(&c.name))
                .collect::<Vec<_>>()
                .join(", ")
        };

        let mut sql = format!("SELECT {col_list} FROM {fq}");
        if let Some(ref w) = query.where_clause {
            let wt = w.trim();
            if !wt.is_empty() {
                sql.push_str(&format!(" WHERE {wt}"));
            }
        }
        if let Some(ref sc) = query.sort_col {
            let dir = if query.sort_asc { "ASC" } else { "DESC" };
            sql.push_str(&format!(" ORDER BY {} {dir}", quote_ident(sc)));
        }
        sql.push_str(&format!(
            " LIMIT {} OFFSET {}",
            query.limit.saturating_add(1),
            query.offset
        ));

        let mut has_more = false;
        let rows = conn
            .query_fold(&sql, Vec::new(), |mut rows, row| {
                if rows.len() < query.limit {
                    rows.push(into_cells(row));
                } else {
                    has_more = true;
                }
                rows
            })
            .await
            .map_err(|e| AdapterError::Query(e.to_string()))?;

        let est: Option<Row> = conn
            .exec_first(
                "SELECT COALESCE(TABLE_ROWS, 0) FROM information_schema.TABLES \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
                (&query.schema, &query.table),
            )
            .await
            .ok()
            .flatten();
        let total_estimate = est
            .map(|r| row_i64(&r, 0))
            .unwrap_or(rows.len() as i64)
            .max(rows.len() as i64);

        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(TablePage {
            rows,
            has_more,
            total_estimate,
            elapsed_ms,
        })
    }

    async fn execute_sql(&self, sql: &str, read_only_override: bool) -> AdapterResult<QueryResult> {
        let enforce_ro = self.config.read_only || read_only_override;
        if enforce_ro && !is_read_only_sql(sql) {
            return Err(AdapterError::ReadOnlyViolation);
        }

        let started = Instant::now();
        let mut conn = self.get_conn().await?;

        let mut result_set = conn
            .query_iter(sql)
            .await
            .map_err(|e| AdapterError::Query(e.to_string()))?;

        let cols_arc = result_set.columns_ref();
        let columns: Vec<String> = cols_arc
            .iter()
            .map(|c| c.name_str().to_string())
            .collect();
        let column_types: Vec<String> = cols_arc
            .iter()
            .map(|c| format!("{:?}", c.column_type()).to_lowercase())
            .collect();

        let affected_rows = result_set.affected_rows();
        let rows = result_set
            .map(into_cells)
            .await
            .map_err(|e| AdapterError::Query(e.to_string()))?;

        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let message = if !columns.is_empty() {
            format!("返回 {} 行，耗时 {:.1}ms", rows.len(), elapsed_ms)
        } else {
            format!("执行成功，影响 {affected_rows} 行，耗时 {:.1}ms", elapsed_ms)
        };

        Ok(QueryResult {
            columns,
            column_types,
            rows,
            affected_rows,
            elapsed_ms,
            message,
        })
    }

    async fn explain_sql(&self, sql: &str) -> AdapterResult<ExplainPlan> {
        let mut conn = self.get_conn().await?;
        let trimmed = sql.trim().trim_end_matches(';');
        let explain_q = format!("EXPLAIN FORMAT=JSON {trimmed}");
        let row: Option<Row> = conn
            .query_first(&explain_q)
            .await
            .map_err(|e| AdapterError::Query(e.to_string()))?;

        let raw = row
            .and_then(|r| row_str(&r, 0))
            .unwrap_or_else(|| "{}".to_string());
        Ok(parse_mysql_explain_json(&raw))
    }

    async fn commit_staged_changes(&self, changes: &[StagedChange]) -> AdapterResult<u64> {
        if self.config.read_only {
            return Err(AdapterError::ReadOnlyViolation);
        }
        if changes.is_empty() {
            return Ok(0);
        }

        let mut conn = self.get_conn().await?;
        let mut tx = conn
            .start_transaction(TxOpts::default())
            .await
            .map_err(|e| AdapterError::Query(e.to_string()))?;

        let mut affected = 0u64;
        for ch in changes {
            let fq = format!("{}.{}", quote_ident(&ch.schema), quote_ident(&ch.table));
            let stmt = match &ch.kind {
                StagedChangeKind::Update { col, new_val, .. } => {
                    let pk_val = ch.pk_val.as_ref().ok_or_else(|| {
                        AdapterError::Query("Missing primary key for UPDATE".into())
                    })?;
                    format!(
                        "UPDATE {fq} SET {} = {} WHERE {} = {}",
                        quote_ident(col),
                        sql_literal(new_val),
                        quote_ident(&ch.pk_col),
                        sql_literal(&Some(pk_val.clone()))
                    )
                }
                StagedChangeKind::Insert { values } => {
                    if values.is_empty() {
                        format!("INSERT INTO {fq} () VALUES ()")
                    } else {
                        let cols: Vec<String> = values.keys().map(|k| quote_ident(k)).collect();
                        let vals: Vec<String> = values.values().map(sql_literal).collect();
                        format!(
                            "INSERT INTO {fq} ({}) VALUES ({})",
                            cols.join(", "),
                            vals.join(", ")
                        )
                    }
                }
                StagedChangeKind::Delete => {
                    let pk_val = ch.pk_val.as_ref().ok_or_else(|| {
                        AdapterError::Query("Missing primary key for DELETE".into())
                    })?;
                    format!(
                        "DELETE FROM {fq} WHERE {} = {}",
                        quote_ident(&ch.pk_col),
                        sql_literal(&Some(pk_val.clone()))
                    )
                }
            };

            tx.query_drop(&stmt)
                .await
                .map_err(|e| AdapterError::Query(e.to_string()))?;
            affected += tx.affected_rows().max(1);
        }

        tx.commit()
            .await
            .map_err(|e| AdapterError::Query(e.to_string()))?;
        Ok(affected)
    }

    async fn get_server_metrics(&self) -> AdapterResult<ServerMetrics> {
        let mut conn = self.get_conn().await?;

        // max_connections
        let max_conn_row: Option<Row> = conn
            .query_first("SHOW GLOBAL VARIABLES LIKE 'max_connections'")
            .await
            .ok()
            .flatten();
        let max_connections = max_conn_row
            .and_then(|r| row_str(&r, 1))
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(151);

        // Processlist / Sessions
        let proc_rows: Vec<Row> = conn
            .query("SHOW FULL PROCESSLIST")
            .await
            .unwrap_or_default();
        let mut active_count = 0i64;
        let mut idle_count = 0i64;
        let mut waiting_count = 0i64;
        let mut sessions = Vec::new();
        for r in &proc_rows {
            let pid = row_i64(r, 0) as i32;
            let username = row_str(r, 1).unwrap_or_default();
            let command = row_str(r, 4).unwrap_or_default();
            let time_s = row_i64(r, 5);
            let state_raw = row_str(r, 6).unwrap_or_default();
            let info = row_str(r, 7).unwrap_or_default();

            if command.eq_ignore_ascii_case("Sleep") {
                idle_count += 1;
            } else if state_raw.to_lowercase().contains("lock")
                || state_raw.to_lowercase().contains("wait")
            {
                waiting_count += 1;
            } else {
                active_count += 1;
            }

            sessions.push(SessionInfo {
                pid,
                username,
                state: if state_raw.is_empty() {
                    command
                } else {
                    state_raw
                },
                duration: format!("{time_s}s"),
                query: if info.is_empty() {
                    "<idle>".to_string()
                } else {
                    info
                },
            });
        }
        let active_connections = proc_rows.len() as i64;

        // Buffer pool cache hit ratio + QPS/TPS estimate
        let status_rows: Vec<Row> = conn
            .query(
                "SHOW GLOBAL STATUS WHERE Variable_name IN (\
                 'Innodb_buffer_pool_read_requests', 'Innodb_buffer_pool_reads', \
                 'Questions', 'Uptime')",
            )
            .await
            .unwrap_or_default();
        let mut status_map: HashMap<String, f64> = HashMap::new();
        for r in &status_rows {
            if let (Some(k), Some(v)) = (row_str(r, 0), row_str(r, 1)) {
                if let Ok(n) = v.parse::<f64>() {
                    status_map.insert(k, n);
                }
            }
        }
        let read_reqs = *status_map
            .get("Innodb_buffer_pool_read_requests")
            .unwrap_or(&0.0);
        let reads = *status_map.get("Innodb_buffer_pool_reads").unwrap_or(&0.0);
        let cache_hit_ratio = if read_reqs > 0.0 {
            ((read_reqs - reads).max(0.0) / read_reqs * 100.0).min(100.0)
        } else {
            100.0
        };
        let questions = *status_map.get("Questions").unwrap_or(&0.0);
        let uptime = (*status_map.get("Uptime").unwrap_or(&1.0)).max(1.0);
        let tps = (questions / uptime).round() as i64;

        // Storage & top tables
        let size_rows: Vec<Row> = conn
            .query(
                "SELECT TABLE_SCHEMA, TABLE_NAME, \
                        COALESCE(DATA_LENGTH, 0), COALESCE(INDEX_LENGTH, 0) \
                 FROM information_schema.TABLES \
                 WHERE TABLE_SCHEMA NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys') \
                 ORDER BY (COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0)) DESC \
                 LIMIT 10",
            )
            .await
            .unwrap_or_default();

        let mut total_data = 0i64;
        let mut total_index = 0i64;
        let mut top_tables = Vec::new();
        for r in &size_rows {
            let sch = row_str(r, 0).unwrap_or_default();
            let name = row_str(r, 1).unwrap_or_default();
            let d = row_i64(r, 2);
            let ix = row_i64(r, 3);
            total_data += d;
            total_index += ix;
            top_tables.push(TableStorageInfo {
                schema: sch,
                name,
                total_size: format_bytes(d + ix),
                data_size: format_bytes(d),
                index_size: format_bytes(ix),
            });
        }

        Ok(ServerMetrics {
            active_connections,
            max_connections,
            active_count,
            idle_count,
            waiting_count,
            cache_hit_ratio,
            db_size: format_bytes(total_data + total_index),
            data_size: format_bytes(total_data),
            index_size: format_bytes(total_index),
            tps,
            avg_latency_ms: 0.0,
            sessions,
            top_tables,
        })
    }

    async fn cancel_session(&self, pid: i32, terminate: bool) -> AdapterResult<bool> {
        let mut conn = self.get_conn().await?;
        let stmt = if terminate {
            format!("KILL {pid}")
        } else {
            format!("KILL QUERY {pid}")
        };
        conn.query_drop(&stmt)
            .await
            .map_err(|e| AdapterError::Query(e.to_string()))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mysql_uri_parsing_and_adapter_factory() {
        let cfg = ConnectionConfig::from_uri(
            "mysql://root:pass123@127.0.0.1:3307/shop_db?sslmode=disable",
        )
        .unwrap();
        assert_eq!(cfg.driver, DbDriver::Mysql);
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 3307);
        assert_eq!(cfg.database, "shop_db");
        assert_eq!(cfg.username, "root");
        assert_eq!(cfg.password, "pass123");
        assert_eq!(cfg.ssl_mode, SslMode::Disable);
        assert!(create_adapter(cfg).is_ok());
    }

    #[test]
    fn test_mysql_staged_sql_and_enum_parsing() {
        let changes = vec![StagedChange {
            schema: "shop".into(),
            table: "orders".into(),
            row_idx: 0,
            pk_col: "id".into(),
            pk_val: Some("101".into()),
            kind: StagedChangeKind::Update {
                col: "status".into(),
                old_val: Some("pending".into()),
                new_val: Some("paid".into()),
            },
        }];
        let sql = generate_mysql_staged_sql(&changes);
        assert!(sql.starts_with("START TRANSACTION;"));
        assert!(sql.contains("UPDATE `shop`.`orders` SET `status` = 'paid' WHERE `id` = '101';"));
        assert!(sql.ends_with("COMMIT;"));

        assert_eq!(
            parse_mysql_enum_values("enum('active','pending','canceled')"),
            vec!["active", "pending", "canceled"]
        );
        assert!(is_read_only_sql("SHOW TABLES"));
        assert!(is_read_only_sql("DESCRIBE users"));
        assert!(!is_read_only_sql("DROP TABLE users"));
    }
}
