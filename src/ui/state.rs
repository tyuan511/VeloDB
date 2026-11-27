use crate::adapter::{
    ColumnDef, ConnectionConfig, DatabaseAdapter, DbDriver, ExplainPlan, ExportFormat,
    QueryResult, SchemaObjectType, ServerMetrics, SslMode, StagedChange, TableStructure,
};
use crate::config::sync::SyncConfig;
use crate::config::{AppSettings, QueryHistoryEntry, SqlSnippet};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKind {
    Table,
    Sql,
    Monitor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableSubview {
    Data,
    Structure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructTab {
    Columns,
    Indexes,
    Constraints,
    Ddl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlResultTab {
    Result,
    Explain,
    Messages,
    History,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawerTab {
    Cell,
    Row,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectorViewMode {
    Editor,
    Tree,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsCategory {
    General,
    Grid,
    Editor,
    Safety,
    Keys,
    Mcp,
    Sync,
    Advanced,
}

impl SettingsCategory {
    pub fn title(&self) -> &'static str {
        match self {
            Self::General => "常规与外观",
            Self::Grid => "数据网格与格式",
            Self::Editor => "SQL 编辑器",
            Self::Safety => "安全与查询",
            Self::Keys => "快捷键",
            Self::Mcp => "MCP 服务",
            Self::Sync => "云端配置同步",
            Self::Advanced => "高级与配置",
        }
    }

    pub fn desc(&self) -> &'static str {
        match self {
            Self::General => "主题、字体、毛玻璃与界面密度（调整后实时生效）",
            Self::Grid => "表格默认分页、NULL 展示、时区与数值格式化",
            Self::Editor => "代码字号、行号、缩进与执行行为",
            Self::Safety => "安全模式策略、生产库保护与超时控制",
            Self::Keys => "快捷键方案、Vim 模式与常用快捷键一览",
            Self::Mcp => "本地 MCP 服务端开关、只读安全锁与暴露工具",
            Self::Sync => "基于 S3 兼容存储或 WebDAV 同步全部主机连接与偏好设置",
            Self::Advanced => "导出 settings.toml、重置偏好与清理本地缓存",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableDangerAction {
    Truncate,
    Drop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActiveModal {
    None,
    CommandPalette,
    ConnectionManager,
    NewConnection { step: u8 },
    McpConfig,
    ColumnsPicker,
    PreviewSql,
    ExportImport { is_import: bool },
    SnippetsManager,
    Settings,
    RenameTable { schema: String, table: String },
    ConfirmTableAction {
        action: TableDangerAction,
        schema: String,
        table: String,
    },
}

#[derive(Clone, Debug)]
pub struct TableTabState {
    pub schema: String,
    pub table: String,
    pub subview: TableSubview,
    pub struct_tab: StructTab,
    pub columns: Arc<[ColumnDef]>,
    pub rows: Vec<Vec<Option<String>>>,
    pub structure: Option<Arc<TableStructure>>,
    pub ddl_editor: gpui::Entity<gpui_component::input::EditorState>,
    pub ddl_formatted: bool,
    pub ddl_soft_wrap: bool,
    pub query_generation: u64,
    pub total_estimate: i64,
    pub has_next_page: bool,
    pub page_error: Option<String>,
    pub elapsed_ms: f64,
    pub where_clause: String,
    pub applied_where_clause: String,
    pub sort_col: Option<String>,
    pub sort_asc: bool,
    pub limit: usize,
    pub page: usize,
    pub hidden_cols: HashSet<String>,
    pub selected_row: usize,
    pub selected_col_idx: usize,
    pub checked_rows: HashSet<usize>,
    pub added_rows: HashSet<usize>,
    pub deleted_rows: HashSet<usize>,
    pub editing_cell: Option<(usize, usize)>,
    pub is_loading: bool,
    pub col_widths: HashMap<String, f32>,
    pub vertical_scroll: gpui::UniformListScrollHandle,
    pub horizontal_scroll: gpui::ScrollHandle,
}

#[derive(Clone, Debug)]
pub struct SqlTabState {
    pub editor: gpui::Entity<gpui_component::input::EditorState>,
    pub result_tab: SqlResultTab,
    pub result: Option<QueryResult>,
    pub explainable_sql: Option<Arc<str>>,
    pub explain: Option<ExplainPlan>,
    pub messages: Vec<String>,
    pub show_snippets_menu: bool,
    pub is_loading: bool,
    pub selected_row: Option<usize>,
    pub selected_col_idx: Option<usize>,
    pub col_widths: HashMap<usize, f32>,
    pub vertical_scroll: gpui::UniformListScrollHandle,
    pub horizontal_scroll: gpui::ScrollHandle,
}

#[derive(Clone, Debug)]
pub struct TabItem {
    pub id: String,
    pub title: String,
    pub kind: TabKind,
    pub table_state: Option<TableTabState>,
    pub sql_state: Option<SqlTabState>,
}

impl TabItem {
    pub(super) fn cell_value(&self, row: usize, column: usize) -> Option<&str> {
        let rows = match self.kind {
            TabKind::Table => &self.table_state.as_ref()?.rows,
            TabKind::Sql => &self.sql_state.as_ref()?.result.as_ref()?.rows,
            TabKind::Monitor => return None,
        };
        rows.get(row)?.get(column)?.as_deref()
    }
}

#[derive(Clone, Debug)]
pub struct NewConnForm {
    pub editing_id: Option<String>,
    pub return_to_manager: bool,
    pub driver: DbDriver,
    pub uri: String,
    pub name: String,
    pub tag: String,
    pub color: String,
    pub host: String,
    pub port: String,
    pub database: String,
    pub username: String,
    pub password: String,
    pub sqlite_path: String,
    pub ssl_mode: SslMode,
    pub ssh_enabled: bool,
    pub ssh_host: String,
    pub ssh_port: String,
    pub ssh_user: String,
    pub ssh_auth: String,
    pub ssh_key_path: String,
    pub read_only: bool,
    pub test_status: Option<String>,
}

impl Default for NewConnForm {
    fn default() -> Self {
        Self {
            editing_id: None,
            return_to_manager: false,
            driver: DbDriver::Postgres,
            uri: String::new(),
            name: "local_postgres".to_string(),
            tag: "本地".to_string(),
            color: "#3b82f6".to_string(),
            host: "127.0.0.1".to_string(),
            port: "5432".to_string(),
            database: "postgres".to_string(),
            username: "postgres".to_string(),
            password: String::new(),
            sqlite_path: "/Users/yuantang/data/app.sqlite3".to_string(),
            ssl_mode: SslMode::Prefer,
            ssh_enabled: false,
            ssh_host: "bastion.internal".to_string(),
            ssh_port: "22".to_string(),
            ssh_user: "ops".to_string(),
            ssh_auth: "key".to_string(),
            ssh_key_path: "~/.ssh/id_ed25519".to_string(),
            read_only: false,
            test_status: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ColResizeTarget {
    Table { col_name: String },
    Sql { col_idx: usize },
}

#[derive(Clone, Debug)]
pub struct ColResizeState {
    pub target: ColResizeTarget,
    pub start_x: f32,
    pub start_width: f32,
}

#[derive(Clone, Debug)]
pub struct ContextMenuState {
    pub open: bool,
    pub x: f32,
    pub y: f32,
    pub row_idx: usize,
    pub col_idx: usize,
    pub is_sql: bool,
}

#[derive(Clone, Debug)]
pub struct SidebarContextMenu {
    pub open: bool,
    pub x: f32,
    pub y: f32,
    pub schema: String,
    pub name: String,
    pub object_type: SchemaObjectType,
    pub can_open: bool,
}

impl Default for SidebarContextMenu {
    fn default() -> Self {
        Self {
            open: false,
            x: 0.0,
            y: 0.0,
            schema: String::new(),
            name: String::new(),
            object_type: SchemaObjectType::Table,
            can_open: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TabContextMenu {
    pub open: bool,
    pub x: f32,
    pub y: f32,
    pub tab_id: String,
}

pub struct AppData {
    pub is_connected: bool,
    pub connections: Vec<ConnectionConfig>,
    pub active_conn: Option<ConnectionConfig>,
    pub adapter: Option<Arc<dyn DatabaseAdapter>>,
    pub server_version: String,
    pub rtt_ms: u64,

    pub sidebar_open: bool,
    pub sidebar_resizing: bool,
    pub inspector_resizing: bool,
    pub sql_split_resizing: Option<(f32, f32)>,
    pub col_resizing: Option<ColResizeState>,
    pub current_db: String,
    pub databases: Vec<String>,
    pub db_dropdown_open: bool,
    pub current_schema: String,
    pub schemas: Vec<String>,
    pub schema_dropdown_open: bool,
    pub schema_objects: Arc<BTreeMap<String, Vec<crate::adapter::SchemaObject>>>,
    pub collapsed_groups: HashSet<String>,
    pub favorites: HashSet<String>,
    pub sidebar_filter: String,
    pub sidebar_favorites_only: bool,

    pub tabs: Vec<TabItem>,
    pub active_tab_id: String,

    pub staged_changes: Vec<StagedChange>,
    pub drawer_open: bool,
    pub drawer_tab: DrawerTab,
    pub inspector_view_mode: InspectorViewMode,
    pub inspector_soft_wrap: bool,
    pub inspector_is_null: bool,
    pub inspector_collapsed_paths: HashSet<String>,

    pub monitor_metrics: Option<ServerMetrics>,
    pub settings: AppSettings,
    pub settings_cat: SettingsCategory,
    pub sync_config: SyncConfig,
    pub sync_busy: bool,
    pub sync_status_msg: Option<String>,
    pub snippets: Vec<SqlSnippet>,
    pub history: Vec<QueryHistoryEntry>,

    pub active_modal: ActiveModal,
    pub open_dropdown: Option<&'static str>,
    pub cmd_query: String,
    pub cmd_selected_idx: usize,
    pub export_fmt: ExportFormat,
    pub new_conn: NewConnForm,
    pub ctx_menu: ContextMenuState,
    pub sidebar_ctx_menu: SidebarContextMenu,
    pub tab_ctx_menu: TabContextMenu,
    pub toast: Option<String>,
}
