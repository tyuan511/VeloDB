pub mod mysql;
pub mod postgres;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("Connection error: {0}")]
    Connection(String),
    #[error("Query error: {0}")]
    Query(String),
    #[error("Unsupported driver: {0}")]
    UnsupportedDriver(String),
    #[error("Read-only mode enabled: write/DDL operations are blocked")]
    ReadOnlyViolation,
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

pub type AdapterResult<T> = Result<T, AdapterError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbDriver {
    Postgres,
    Mysql,
    Sqlite,
    Redis,
    Clickhouse,
    Mariadb,
}

impl DbDriver {
    pub fn display_name(&self) -> &'static str {
        match self {
            DbDriver::Postgres => "PostgreSQL",
            DbDriver::Mysql => "MySQL",
            DbDriver::Sqlite => "SQLite",
            DbDriver::Redis => "Redis",
            DbDriver::Clickhouse => "ClickHouse",
            DbDriver::Mariadb => "MariaDB",
        }
    }

    pub fn default_port(&self) -> u16 {
        match self {
            DbDriver::Postgres => 5432,
            DbDriver::Mysql => 3306,
            DbDriver::Sqlite => 0,
            DbDriver::Redis => 6379,
            DbDriver::Clickhouse => 8123,
            DbDriver::Mariadb => 3306,
        }
    }

    pub fn default_db(&self) -> &'static str {
        match self {
            DbDriver::Postgres => "postgres",
            DbDriver::Mysql | DbDriver::Mariadb => "",
            DbDriver::Sqlite => "app.sqlite3",
            DbDriver::Redis => "0",
            DbDriver::Clickhouse => "default",
        }
    }

    pub fn uri_placeholder(&self) -> &'static str {
        match self {
            DbDriver::Postgres => "postgresql://user:pass@localhost:5432/postgres",
            DbDriver::Mysql => "mysql://root:pass@localhost:3306/app_db",
            DbDriver::Sqlite => "sqlite:///path/to/app.sqlite3",
            DbDriver::Redis => "redis://default:pass@localhost:6379/0",
            DbDriver::Clickhouse => "http://default@localhost:8123/default",
            DbDriver::Mariadb => "mariadb://root:pass@localhost:3306/app_db",
        }
    }

    pub fn default_user(&self) -> &'static str {
        match self {
            DbDriver::Postgres => "postgres",
            DbDriver::Mysql | DbDriver::Mariadb => "root",
            DbDriver::Sqlite => "",
            DbDriver::Redis | DbDriver::Clickhouse => "default",
        }
    }

    pub fn is_supported(&self) -> bool {
        matches!(self, DbDriver::Postgres | DbDriver::Mysql)
    }

    pub fn is_file_based(&self) -> bool {
        matches!(self, DbDriver::Sqlite)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    Disable,
    Prefer,
    Require,
    VerifyFull,
}

impl Default for SslMode {
    fn default() -> Self {
        SslMode::Prefer
    }
}

impl SslMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SslMode::Disable => "disable",
            SslMode::Prefer => "prefer",
            SslMode::Require => "require",
            SslMode::VerifyFull => "verify-full",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SshTunnelConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: String, // "key" | "password"
    pub private_key_path: String,
    pub passphrase: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: String,
    pub driver: DbDriver,
    pub name: String,
    pub env: String,   // "prod" | "staging" | "local" | "custom"
    pub label: String, // e.g. "生产", "预发", "本地", "数据分析"
    pub color: String, // hex e.g. "#ef4444"
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub file_path: Option<String>,
    pub ssl_mode: SslMode,
    pub ssh: Option<SshTunnelConfig>,
    pub read_only: bool,
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex, 16) {
                    out.push(val);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl ConnectionConfig {
    pub fn display_host(&self) -> String {
        if self.driver.is_file_based() {
            self.file_path.clone().unwrap_or_else(|| self.database.clone())
        } else if self.database.is_empty() {
            format!("{}:{}", self.host, self.port)
        } else {
            format!("{}:{}/{}", self.host, self.port, self.database)
        }
    }

    pub fn from_uri(uri_str: &str) -> AdapterResult<Self> {
        let parsed = url::Url::parse(uri_str.trim())
            .map_err(|e| AdapterError::InvalidConfig(format!("Invalid URI: {e}")))?;
        let scheme = parsed.scheme();
        let driver = match scheme {
            "postgres" | "postgresql" => DbDriver::Postgres,
            "mysql" => DbDriver::Mysql,
            other => return Err(AdapterError::UnsupportedDriver(other.to_string())),
        };
        let host = parsed.host_str().unwrap_or("localhost").to_string();
        let port = parsed.port().unwrap_or_else(|| driver.default_port());
        let database = percent_decode(parsed.path().trim_start_matches('/'));
        let database = if database.is_empty() {
            driver.default_db().to_string()
        } else {
            database
        };
        let username = if parsed.username().is_empty() {
            driver.default_user().to_string()
        } else {
            percent_decode(parsed.username())
        };
        let password = parsed.password().map(percent_decode).unwrap_or_default();
        let mut ssl_mode = SslMode::Prefer;
        for (k, v) in parsed.query_pairs() {
            if k == "sslmode" || k == "ssl-mode" {
                ssl_mode = match v.to_lowercase().as_str() {
                    "disable" | "disabled" => SslMode::Disable,
                    "require" | "required" => SslMode::Require,
                    "verify-full" | "verify_identity" => SslMode::VerifyFull,
                    _ => SslMode::Prefer,
                };
            }
        }
        let is_local = host == "localhost" || host == "127.0.0.1";
        let conn_name = if database.is_empty() {
            format!("{host}:{port}")
        } else {
            format!("{database}@{host}")
        };
        Ok(Self {
            id: format!("conn-{}", uuid::Uuid::new_v4().simple()),
            driver,
            name: conn_name,
            env: if is_local { "local" } else { "custom" }.into(),
            label: if is_local { "本地" } else { "自定义" }.into(),
            color: if is_local { "#10b981" } else { "#3b82f6" }.into(),
            host,
            port,
            database,
            username,
            password,
            file_path: None,
            ssl_mode,
            ssh: None,
            read_only: false,
        })
    }

    pub fn from_postgres_uri(uri_str: &str) -> AdapterResult<Self> {
        let cfg = Self::from_uri(uri_str)?;
        if cfg.driver != DbDriver::Postgres {
            return Err(AdapterError::UnsupportedDriver(
                cfg.driver.display_name().to_string(),
            ));
        }
        Ok(cfg)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionTestInfo {
    pub server_version: String,
    pub rtt_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchemaObjectType {
    Table,
    View,
    MaterializedView,
    Function,
    Sequence,
    Enum,
    Extension,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaObject {
    pub object_type: SchemaObjectType,
    pub name: String,
    pub meta: String,
    pub schema: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaObjects {
    pub schemas: Vec<String>,
    pub groups: BTreeMap<String, Vec<SchemaObject>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDef {
    pub name: String,
    pub data_type: String,
    pub is_pk: bool,
    pub fk_table: Option<String>,
    pub fk_column: Option<String>,
    pub nullable: bool,
    pub default_expr: Option<String>,
    pub comment: Option<String>,
    pub enum_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDef {
    pub name: String,
    pub columns: String,
    pub method: String,
    pub is_unique: bool,
    pub size: String,
    pub scans: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintDef {
    pub name: String,
    pub constraint_type: String,
    pub definition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableStructure {
    pub schema: String,
    pub table: String,
    pub columns: Arc<[ColumnDef]>,
    pub indexes: Vec<IndexDef>,
    pub constraints: Vec<ConstraintDef>,
    pub ddl: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableDataQuery {
    pub schema: String,
    pub table: String,
    pub where_clause: Option<String>,
    pub sort_col: Option<String>,
    pub sort_asc: bool,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableDataResult {
    pub columns: Arc<[ColumnDef]>,
    pub rows: Vec<Vec<Option<String>>>,
    pub total_estimate: i64,
    pub elapsed_ms: f64,
}

pub struct TablePage {
    pub rows: Vec<Vec<Option<String>>>,
    pub has_more: bool,
    pub total_estimate: i64,
    pub elapsed_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub column_types: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub affected_rows: u64,
    pub elapsed_ms: f64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainNode {
    pub depth: usize,
    pub operation: String,
    pub detail: String,
    pub actual_rows: String,
    pub time_pct: f64,
    pub time_ms: String,
    pub is_warn: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainPlan {
    pub total_time_ms: f64,
    pub planning_time_ms: f64,
    pub shared_hit_blocks: i64,
    pub warnings: usize,
    pub nodes: Vec<ExplainNode>,
    pub raw_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StagedChangeKind {
    Update {
        col: String,
        old_val: Option<String>,
        new_val: Option<String>,
    },
    Insert {
        values: BTreeMap<String, Option<String>>,
    },
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedChange {
    pub schema: String,
    pub table: String,
    pub row_idx: usize,
    pub pk_col: String,
    pub pk_val: Option<String>,
    pub kind: StagedChangeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub pid: i32,
    pub username: String,
    pub state: String,
    pub duration: String,
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableStorageInfo {
    pub schema: String,
    pub name: String,
    pub total_size: String,
    pub data_size: String,
    pub index_size: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerMetrics {
    pub active_connections: i64,
    pub max_connections: i64,
    pub active_count: i64,
    pub idle_count: i64,
    pub waiting_count: i64,
    pub cache_hit_ratio: f64,
    pub db_size: String,
    pub data_size: String,
    pub index_size: String,
    pub tps: i64,
    pub avg_latency_ms: f64,
    pub sessions: Vec<SessionInfo>,
    pub top_tables: Vec<TableStorageInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Json,
    SqlInsert,
}

/// Unified Database Adapter abstraction layer.
/// Currently implemented by `PostgresAdapter`; future engines (MySQL, SQLite, Redis, ClickHouse, MariaDB)
/// implement this trait to plug directly into VeloDB's workspace & MCP server.
#[async_trait]
pub trait DatabaseAdapter: Send + Sync {
    fn driver(&self) -> DbDriver;
    fn config(&self) -> &ConnectionConfig;

    async fn test_connection(&self) -> AdapterResult<ConnectionTestInfo>;
    async fn list_databases(&self) -> AdapterResult<Vec<String>>;
    async fn list_schemas(&self) -> AdapterResult<Vec<String>>;
    async fn list_schema_objects(&self, schema: &str) -> AdapterResult<SchemaObjects>;
    async fn get_table_structure(&self, schema: &str, table: &str) -> AdapterResult<TableStructure>;
    async fn query_table_page(&self, query: &TableDataQuery, columns: &[ColumnDef]) -> AdapterResult<TablePage>;
    async fn query_table_data(&self, query: &TableDataQuery) -> AdapterResult<TableDataResult> {
        let started = std::time::Instant::now();
        let structure = self.get_table_structure(&query.schema, &query.table).await?;
        let page = self.query_table_page(query, &structure.columns).await?;
        Ok(TableDataResult {
            columns: structure.columns,
            rows: page.rows,
            total_estimate: page.total_estimate,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }
    async fn execute_sql(&self, sql: &str, read_only_override: bool) -> AdapterResult<QueryResult>;
    async fn explain_sql(&self, sql: &str) -> AdapterResult<ExplainPlan>;
    async fn commit_staged_changes(&self, changes: &[StagedChange]) -> AdapterResult<u64>;
    async fn get_server_metrics(&self) -> AdapterResult<ServerMetrics>;
    async fn cancel_session(&self, pid: i32, terminate: bool) -> AdapterResult<bool>;
}

/// Factory function creating the appropriate `DatabaseAdapter` for the given `ConnectionConfig`.
pub fn create_adapter(config: ConnectionConfig) -> AdapterResult<Arc<dyn DatabaseAdapter>> {
    match config.driver {
        DbDriver::Postgres => Ok(Arc::new(postgres::PostgresAdapter::new(config)?)),
        DbDriver::Mysql => Ok(Arc::new(mysql::MySqlAdapter::new(config)?)),
        other => Err(AdapterError::UnsupportedDriver(format!(
            "{} adapter is not implemented yet (currently PostgreSQL & MySQL)",
            other.display_name()
        ))),
    }
}
