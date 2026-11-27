use crate::adapter::{
    create_adapter, ConnectionConfig, DbDriver, ExportFormat, SchemaObjects, SshTunnelConfig,
    StagedChange, StagedChangeKind, TableDataQuery,
};
use crate::config::sync::{
    load_sync_config, merge_connections, merge_favorites, merge_snippets, pull_snapshot,
    push_snapshot, save_sync_config, test_sync_connection, SyncConfig, SyncSnapshot,
};
use crate::config::{
    load_connections, load_favorites, load_history, load_settings, load_snippets, save_connections,
    save_favorites, save_history, save_settings, save_snippets, QueryHistoryEntry,
};
use crate::ui::state::{
    ActiveModal, AppData, ContextMenuState, DrawerTab, InspectorViewMode, NewConnForm,
    SettingsCategory, SidebarContextMenu, SqlResultTab, SqlTabState, StructTab, TabContextMenu,
    TabItem, TabKind, TableDangerAction, TableSubview, TableTabState,
};
use crate::ui::icons::hgi;
use crate::ui::sidebar::{build_sidebar_tooltip, SIDEBAR_TOOLTIP_DELAY};
use crate::ui::sql_lsp::{SqlLspCatalog, SqlLspProvider};
use crate::ui::theme::{with_alpha, VeloTheme};
use gpui::{
    div, prelude::*, px, AnyElement, App, Context, Entity, FocusHandle, Focusable,
    InteractiveElement, KeyDownEvent, MouseButton, MouseMoveEvent, ParentElement, Render, Styled,
    Window,
};
use gpui_component::input::{EditorState, InputEvent, InputState};
use gpui_component::{
    ActiveTheme as _, Theme as ComponentTheme, ThemeMode as ComponentThemeMode, TitleBar,
};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use tokio::runtime::Handle as TokioHandle;

// Animate in a separate view so each frame only invalidates the progress bar.
struct ConnectionProgressBar {
    label: gpui::SharedString,
}

impl Render for ConnectionProgressBar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui_component::progress::Progress::new("connection-progress")
            .accessibility_label(self.label.clone())
            .h(px(4.0))
            .loading(true)
    }
}

struct ConnectionLoad {
    label: String,
    bar: Entity<ConnectionProgressBar>,
    task: tokio::task::AbortHandle,
}

impl Drop for ConnectionLoad {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub struct VeloDbApp {
    window_cx: gpui::AsyncWindowContext,
    pub(super) sidebar_tree: Option<Rc<super::sidebar::SidebarTree>>,
    pub(super) sidebar_scroll: gpui::UniformListScrollHandle,
    pending_connection: Option<ConnectionLoad>,
    toast_timer: Option<gpui::Task<()>>,
    session_generation: u64,
    pub rt: TokioHandle,
    pub focus_handle: FocusHandle,
    pub data: AppData,
    pub sql_lsp_catalog: Arc<RwLock<SqlLspCatalog>>,

    // Native GPUI input states
    pub quick_uri_input: Entity<InputState>,
    pub sidebar_filter_input: Entity<InputState>,
    pub where_input: Entity<InputState>,
    pub inspector_editor_input: Entity<EditorState>,
    pub cmd_search_input: Entity<InputState>,
    pub snippet_name_input: Entity<InputState>,
    pub inline_cell_input: Entity<InputState>,
    pub rename_table_input: Entity<InputState>,

    // New connection form inputs
    pub nc_uri_input: Entity<InputState>,
    pub nc_name_input: Entity<InputState>,
    pub nc_tag_input: Entity<InputState>,
    pub nc_host_input: Entity<InputState>,
    pub nc_port_input: Entity<InputState>,
    pub nc_db_input: Entity<InputState>,
    pub nc_user_input: Entity<InputState>,
    pub nc_pass_input: Entity<InputState>,
    pub nc_ssh_host_input: Entity<InputState>,
    pub nc_ssh_port_input: Entity<InputState>,
    pub nc_ssh_user_input: Entity<InputState>,
    pub nc_ssh_key_input: Entity<InputState>,

    // Cloud Sync (S3 / WebDAV) settings inputs
    pub sync_s3_endpoint_input: Entity<InputState>,
    pub sync_s3_region_input: Entity<InputState>,
    pub sync_s3_bucket_input: Entity<InputState>,
    pub sync_s3_key_input: Entity<InputState>,
    pub sync_s3_ak_input: Entity<InputState>,
    pub sync_s3_sk_input: Entity<InputState>,
    pub sync_s3_token_input: Entity<InputState>,
    pub sync_dav_endpoint_input: Entity<InputState>,
    pub sync_dav_user_input: Entity<InputState>,
    pub sync_dav_pass_input: Entity<InputState>,
    pub sync_dav_path_input: Entity<InputState>,
}

impl Focusable for VeloDbApp {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl VeloDbApp {
    pub fn new(rt: TokioHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = load_settings();
        let connections = load_connections();
        let snippets = load_snippets();
        let history = load_history();
        let favorites = load_favorites();
        let sync_config = load_sync_config();

        let quick_uri_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("postgresql://user:pass@localhost:5432/postgres 或 mysql://...")
        });
        let sidebar_filter_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("过滤表、视图、函数...")
        });
        let where_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("输入关键字或条件，如 status = 'active'...")
        });
        let sql_lsp_catalog = Arc::new(RwLock::new(SqlLspCatalog::default()));
        let inspector_editor_input = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(settings.show_line_numbers)
                .folding(true)
                .soft_wrap(true)
                .searchable(true)
                .placeholder("输入文本或 JSON 内容...")
        });
        let cmd_search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("搜索表名、打开监控、切换连接或执行命令...")
        });
        let snippet_name_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("输入片段名称（例如：月活用户统计）")
        });
        let inline_cell_input = cx.new(|cx| InputState::new(window, cx));
        let rename_table_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("输入新的表名...")
        });

        let nc_uri_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("postgresql://user:pass@host:5432/dbname?sslmode=require")
        });
        let nc_name_input = cx.new(|cx| InputState::new(window, cx).default_value("local_postgres"));
        let nc_tag_input = cx.new(|cx| InputState::new(window, cx).default_value("本地"));
        let nc_host_input = cx.new(|cx| InputState::new(window, cx).default_value("127.0.0.1"));
        let nc_port_input = cx.new(|cx| InputState::new(window, cx).default_value("5432"));
        let nc_db_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("可选（留空连接整个实例）")
                .default_value("postgres")
        });
        let nc_user_input = cx.new(|cx| InputState::new(window, cx).default_value("postgres"));
        let nc_pass_input = cx.new(|cx| InputState::new(window, cx).placeholder("••••••••"));
        let nc_ssh_host_input =
            cx.new(|cx| InputState::new(window, cx).default_value("bastion.internal"));
        let nc_ssh_port_input = cx.new(|cx| InputState::new(window, cx).default_value("22"));
        let nc_ssh_user_input = cx.new(|cx| InputState::new(window, cx).default_value("ops"));
        let nc_ssh_key_input =
            cx.new(|cx| InputState::new(window, cx).default_value("~/.ssh/id_ed25519"));

        let s3_init = sync_config.s3.clone();
        let dav_init = sync_config.webdav.clone();
        let sync_s3_endpoint_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://s3.amazonaws.com 或 http://127.0.0.1:9000")
                .default_value(s3_init.endpoint)
        });
        let sync_s3_region_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("us-east-1 或 auto")
                .default_value(s3_init.region)
        });
        let sync_s3_bucket_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("my-velodb-sync")
                .default_value(s3_init.bucket)
        });
        let sync_s3_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("velodb/config-sync.json")
                .default_value(s3_init.object_key)
        });
        let sync_s3_ak_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("AKIA...")
                .default_value(s3_init.access_key_id)
        });
        let sync_s3_sk_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("••••••••")
                .default_value(s3_init.secret_access_key)
        });
        let sync_s3_token_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("可选（临时 STS 凭证）")
                .default_value(s3_init.session_token)
        });
        let sync_dav_endpoint_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://dav.jianguoyun.com/dav/ 或 Nextcloud WebDAV 地址")
                .default_value(dav_init.endpoint)
        });
        let sync_dav_user_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("用户名 / 邮箱")
                .default_value(dav_init.username)
        });
        let sync_dav_pass_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("密码 / 应用专用密码")
                .default_value(dav_init.password)
        });
        let sync_dav_path_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("velodb/config-sync.json")
                .default_value(dav_init.remote_path)
        });

        // Subscribe to filter, WHERE and command palette inputs
        cx.subscribe_in(
            &sidebar_filter_input,
            window,
            |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    this.data.sidebar_filter = input.read(cx).value().to_string();
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.open_first_filtered_object(window, cx);
                }
                _ => {}
            },
        )
        .detach();

        cx.subscribe_in(
            &rename_table_input,
            window,
            |this, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. })
                    && matches!(this.data.active_modal, ActiveModal::RenameTable { .. })
                {
                    this.confirm_rename_table(window, cx);
                }
            },
        )
        .detach();

        cx.subscribe_in(&where_input, window, |this, input, ev: &InputEvent, _, cx| {
            match ev {
                InputEvent::Change => {
                    let val = input.read(cx).value().to_string();
                    if let Some(ts) = this.active_tab_mut().and_then(|t| t.table_state.as_mut()) {
                        ts.where_clause = val;
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.apply_table_filter(input.read(cx).value().to_string(), cx);
                }
                _ => {}
            }
        })
        .detach();

        cx.subscribe_in(&cmd_search_input, window, |this, input, ev: &InputEvent, window, cx| {
            match ev {
                InputEvent::Change => {
                    this.data.cmd_query = input.read(cx).value().to_string();
                    this.data.cmd_selected_idx = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    if this.data.active_modal == ActiveModal::CommandPalette {
                        this.execute_selected_cmd(window, cx);
                    }
                }
                _ => {}
            }
        })
        .detach();

        cx.subscribe_in(&nc_uri_input, window, |this, input, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::Change) {
                let val = input.read(cx).value().to_string();
                if let Ok(parsed) = ConnectionConfig::from_uri(&val) {
                    this.data.new_conn.driver = parsed.driver;
                    if this.data.new_conn.editing_id.is_none() {
                        this.data.new_conn.name = parsed.name.clone();
                        this.data.new_conn.tag = parsed.label.clone();
                    } else {
                        this.data.new_conn.name = this.nc_name_input.read(cx).value().to_string();
                        this.data.new_conn.tag = this.nc_tag_input.read(cx).value().to_string();
                    }
                    this.data.new_conn.host = parsed.host.clone();
                    this.data.new_conn.port = parsed.port.to_string();
                    this.data.new_conn.database = parsed.database.clone();
                    this.data.new_conn.username = parsed.username.clone();
                    this.data.new_conn.password = parsed.password.clone();
                    this.data.new_conn.ssl_mode = parsed.ssl_mode;
                    this.sync_new_conn_inputs(window, cx);
                    cx.notify();
                }
            }
        })
        .detach();

        cx.subscribe_in(&inspector_editor_input, window, |this, ed, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                if ed.read(cx).text().len() > 0 {
                    this.data.inspector_is_null = false;
                }
                this.refresh_inspector_diagnostics(cx);
                cx.notify();
            }
        })
        .detach();

        cx.observe_window_appearance(window, |this, window, cx| {
            this.sync_component_theme(window, cx);
            cx.notify();
        })
        .detach();

        let is_dark_init = match settings.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => matches!(
                window.appearance(),
                gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
            ),
        };
        let target_mode = if is_dark_init {
            ComponentThemeMode::Dark
        } else {
            ComponentThemeMode::Light
        };
        if cx.theme().mode != target_mode {
            ComponentTheme::change(target_mode, None, cx);
        }
        {
            let fs = px(settings.effective_font_size());
            let ct = ComponentTheme::global_mut(cx);
            ct.font_size = fs;
            ct.mono_font_size = fs;
        }

        let data = AppData {
            is_connected: false,
            connections,
            active_conn: None,
            adapter: None,
            server_version: "PostgreSQL".to_string(),
            rtt_ms: 0,
            sidebar_open: true,
            sidebar_resizing: false,
            inspector_resizing: false,
            sql_split_resizing: None,
            col_resizing: None,
            current_db: "postgres".to_string(),
            databases: vec!["postgres".to_string()],
            db_dropdown_open: false,
            current_schema: "public".to_string(),
            schemas: vec!["public".to_string()],
            schema_dropdown_open: false,
            schema_objects: Arc::default(),
            collapsed_groups: HashSet::new(),
            favorites,
            sidebar_filter: String::new(),
            sidebar_favorites_only: false,
            tabs: Vec::new(),
            active_tab_id: String::new(),
            staged_changes: Vec::new(),
            drawer_open: false,
            drawer_tab: DrawerTab::Cell,
            inspector_view_mode: InspectorViewMode::Editor,
            inspector_soft_wrap: true,
            inspector_is_null: false,
            inspector_collapsed_paths: HashSet::new(),
            monitor_metrics: None,
            settings,
            settings_cat: SettingsCategory::General,
            sync_status_msg: sync_config.last_sync_status.clone(),
            sync_config,
            sync_busy: false,
            snippets,
            history,
            active_modal: ActiveModal::None,
            open_dropdown: None,
            cmd_query: String::new(),
            cmd_selected_idx: 0,
            export_fmt: ExportFormat::Csv,
            new_conn: NewConnForm::default(),
            ctx_menu: ContextMenuState {
                open: false,
                x: 0.0,
                y: 0.0,
                row_idx: 0,
                col_idx: 0,
                is_sql: false,
            },
            sidebar_ctx_menu: SidebarContextMenu::default(),
            tab_ctx_menu: TabContextMenu::default(),
            toast: None,
        };

        let app = Self {
            rt,
            window_cx: window.to_async(cx),
            sidebar_tree: None,
            sidebar_scroll: gpui::UniformListScrollHandle::new(),
            pending_connection: None,
            toast_timer: None,
            session_generation: 0,
            focus_handle: cx.focus_handle(),
            data,
            sql_lsp_catalog,
            quick_uri_input,
            sidebar_filter_input,
            where_input,
            inspector_editor_input,
            cmd_search_input,
            snippet_name_input,
            inline_cell_input,
            rename_table_input,
            nc_uri_input,
            nc_name_input,
            nc_tag_input,
            nc_host_input,
            nc_port_input,
            nc_db_input,
            nc_user_input,
            nc_pass_input,
            nc_ssh_host_input,
            nc_ssh_port_input,
            nc_ssh_user_input,
            nc_ssh_key_input,
            sync_s3_endpoint_input,
            sync_s3_region_input,
            sync_s3_bucket_input,
            sync_s3_key_input,
            sync_s3_ak_input,
            sync_s3_sk_input,
            sync_s3_token_input,
            sync_dav_endpoint_input,
            sync_dav_user_input,
            sync_dav_pass_input,
            sync_dav_path_input,
        };
        let mut app = app;
        if app.data.sync_config.enabled
            && app.data.sync_config.auto_sync_on_startup
            && app.data.sync_config.is_configured()
        {
            app.pull_sync_now(true, cx);
        }
        app
    }

    pub fn sync_component_theme(&self, window: &Window, cx: &mut App) {
        let is_dark = match self.data.settings.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => matches!(
                window.appearance(),
                gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
            ),
        };
        let target_mode = if is_dark {
            ComponentThemeMode::Dark
        } else {
            ComponentThemeMode::Light
        };
        if cx.theme().mode != target_mode {
            ComponentTheme::change(target_mode, None, cx);
        }
        let fs = px(self.data.settings.effective_font_size());
        let ct = ComponentTheme::global_mut(cx);
        ct.font_size = fs;
        ct.mono_font_size = fs;
    }

    pub fn theme(&self, window: &Window) -> VeloTheme {
        let is_dark = match self.data.settings.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => matches!(
                window.appearance(),
                gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
            ),
        };
        let conn_color = if self.data.is_connected {
            self.data.active_conn.as_ref().map(|c| c.color.as_str())
        } else {
            None
        };
        VeloTheme::from_mode(
            is_dark,
            self.data.settings.vibrancy,
            conn_color,
            self.data.settings.effective_font_size(),
        )
    }

    pub fn show_toast(&mut self, msg: impl Into<String>, cx: &mut Context<Self>) {
        self.data.toast = Some(msg.into());
        cx.notify();
        self.toast_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(2400))
                .await;
            let _ = this.update(cx, |app, cx| {
                app.data.toast = None;
                cx.notify();
            });
        }));
    }

    pub fn active_tab(&self) -> Option<&TabItem> {
        self.data.tabs.iter().find(|t| t.id == self.data.active_tab_id)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut TabItem> {
        self.data.tabs.iter_mut().find(|t| t.id == self.data.active_tab_id)
    }

    pub fn active_sql_editor(&self) -> Option<&Entity<EditorState>> {
        self.active_tab()?.sql_state.as_ref().map(|tab| &tab.editor)
    }

    pub fn active_ddl_editor(&self) -> Option<&Entity<EditorState>> {
        self.active_tab()?.table_state.as_ref().map(|tab| &tab.ddl_editor)
    }

    pub fn is_any_text_input_focused(&self, window: &Window, cx: &App) -> bool {
        self.quick_uri_input.focus_handle(cx).is_focused(window)
            || self.sidebar_filter_input.focus_handle(cx).is_focused(window)
            || self.where_input.focus_handle(cx).is_focused(window)
            || self.active_sql_editor().is_some_and(|editor| editor.focus_handle(cx).is_focused(window))
            || self.active_ddl_editor().is_some_and(|editor| editor.focus_handle(cx).is_focused(window))
            || self.inspector_editor_input.focus_handle(cx).is_focused(window)
            || self.cmd_search_input.focus_handle(cx).is_focused(window)
            || self.snippet_name_input.focus_handle(cx).is_focused(window)
            || self.inline_cell_input.focus_handle(cx).is_focused(window)
            || self.rename_table_input.focus_handle(cx).is_focused(window)
            || self.nc_uri_input.focus_handle(cx).is_focused(window)
            || self.nc_name_input.focus_handle(cx).is_focused(window)
            || self.nc_tag_input.focus_handle(cx).is_focused(window)
            || self.nc_host_input.focus_handle(cx).is_focused(window)
            || self.nc_port_input.focus_handle(cx).is_focused(window)
            || self.nc_db_input.focus_handle(cx).is_focused(window)
            || self.nc_user_input.focus_handle(cx).is_focused(window)
            || self.nc_pass_input.focus_handle(cx).is_focused(window)
            || self.nc_ssh_host_input.focus_handle(cx).is_focused(window)
            || self.nc_ssh_port_input.focus_handle(cx).is_focused(window)
            || self.nc_ssh_user_input.focus_handle(cx).is_focused(window)
            || self.nc_ssh_key_input.focus_handle(cx).is_focused(window)
            || self.sync_s3_endpoint_input.focus_handle(cx).is_focused(window)
            || self.sync_s3_region_input.focus_handle(cx).is_focused(window)
            || self.sync_s3_bucket_input.focus_handle(cx).is_focused(window)
            || self.sync_s3_key_input.focus_handle(cx).is_focused(window)
            || self.sync_s3_ak_input.focus_handle(cx).is_focused(window)
            || self.sync_s3_sk_input.focus_handle(cx).is_focused(window)
            || self.sync_s3_token_input.focus_handle(cx).is_focused(window)
            || self.sync_dav_endpoint_input.focus_handle(cx).is_focused(window)
            || self.sync_dav_user_input.focus_handle(cx).is_focused(window)
            || self.sync_dav_pass_input.focus_handle(cx).is_focused(window)
            || self.sync_dav_path_input.focus_handle(cx).is_focused(window)
    }

    pub fn save_active_tab_inputs(&mut self, cx: &App) {
        if self.active_tab().is_some_and(|tab| tab.kind == TabKind::Table) {
            let filter = self.where_input.read(cx).text().to_string();
            if let Some(ts) = self.active_tab_mut().and_then(|tab| tab.table_state.as_mut()) {
                ts.where_clause = filter;
            }
        }
    }

    pub(super) fn active_inspector_col_type(&self) -> String {
        let Some(tab) = self.active_tab() else {
            return "text".into();
        };
        match tab.kind {
            TabKind::Table => tab
                .table_state
                .as_ref()
                .and_then(|ts| ts.columns.get(ts.selected_col_idx))
                .map(|c| c.data_type.clone())
                .unwrap_or_else(|| "text".into()),
            TabKind::Sql | TabKind::Monitor => "text".into(),
        }
    }

    pub fn refresh_inspector_diagnostics(&mut self, cx: &mut Context<Self>) {
        if self.data.inspector_is_null {
            self.inspector_editor_input.update(cx, |ed, _| {
                if let Some(diags) = ed.diagnostics_mut() {
                    diags.clear();
                }
            });
            return;
        }
        let col_type = self.active_inspector_col_type();
        let is_json_col = col_type.trim().eq_ignore_ascii_case("json")
            || col_type.trim().eq_ignore_ascii_case("jsonb");
        let raw = self.inspector_editor_input.read(cx).text().to_string();
        let trimmed = raw.trim_start();
        let looks_like_json = trimmed.starts_with('{') || trimmed.starts_with('[');

        if !(is_json_col || looks_like_json) || raw.trim().is_empty() {
            self.inspector_editor_input.update(cx, |ed, cx| {
                if let Some(diags) = ed.diagnostics_mut() {
                    diags.clear();
                }
                if !is_json_col && ed.language_name() != "text" {
                    ed.set_highlighter("text", cx);
                }
            });
            return;
        }

        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(_) => {
                self.inspector_editor_input.update(cx, |ed, cx| {
                    if let Some(diags) = ed.diagnostics_mut() {
                        diags.clear();
                    }
                    if ed.language_name() != "json" {
                        ed.set_highlighter("json", cx);
                    }
                });
            }
            Err(err) => {
                let line = err.line().saturating_sub(1) as u32;
                let col = err.column().saturating_sub(1) as u32;
                let msg = err.to_string();
                self.inspector_editor_input.update(cx, |ed, cx| {
                    if ed.language_name() != "json" {
                        ed.set_highlighter("json", cx);
                    }
                    if let Some(diags) = ed.diagnostics_mut() {
                        diags.clear();
                        diags.push(lsp_types::Diagnostic {
                            range: lsp_types::Range::new(
                                lsp_types::Position::new(line, col),
                                lsp_types::Position::new(line, col.saturating_add(1)),
                            ),
                            severity: Some(lsp_types::DiagnosticSeverity::ERROR),
                            source: Some("json".to_string()),
                            message: msg,
                            ..Default::default()
                        });
                    }
                });
            }
        }
    }

    pub fn sync_inspector_for_active_cell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.data.drawer_open {
            return;
        }
        let col_type = self.active_inspector_col_type();
        let is_json_col = col_type.trim().eq_ignore_ascii_case("json")
            || col_type.trim().eq_ignore_ascii_case("jsonb");
        let cell_val = self.active_tab().and_then(|tab| {
            let (row, column) = match tab.kind {
                TabKind::Table => {
                    let ts = tab.table_state.as_ref()?;
                    (ts.selected_row, ts.selected_col_idx)
                }
                TabKind::Sql => {
                    let ss = tab.sql_state.as_ref()?;
                    (ss.selected_row?, ss.selected_col_idx?)
                }
                TabKind::Monitor => return None,
            };
            tab.cell_value(row, column).map(str::to_owned)
        });

        self.data.inspector_collapsed_paths.clear();

        match cell_val {
            None => {
                self.data.inspector_is_null = true;
                self.data.inspector_view_mode = InspectorViewMode::Editor;
                let lang = if is_json_col { "json" } else { "text" };
                self.inspector_editor_input.update(cx, |s, cx| {
                    s.set_placeholder("NULL", window, cx);
                    s.set_highlighter(lang, cx);
                    s.set_value(String::new(), window, cx);
                    if let Some(diags) = s.diagnostics_mut() {
                        diags.clear();
                    }
                });
            }
            Some(raw) => {
                self.data.inspector_is_null = false;
                let parsed = serde_json::from_str::<serde_json::Value>(&raw);
                let treat_as_json = is_json_col
                    || matches!(
                        parsed,
                        Ok(serde_json::Value::Object(_) | serde_json::Value::Array(_))
                    );
                if treat_as_json {
                    let display_text = match parsed {
                        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| raw.clone()),
                        Err(_) => {
                            self.data.inspector_view_mode = InspectorViewMode::Editor;
                            raw.clone()
                        }
                    };
                    self.inspector_editor_input.update(cx, |s, cx| {
                        s.set_placeholder("输入文本或 JSON 内容...", window, cx);
                        s.set_highlighter("json", cx);
                        s.set_value(display_text, window, cx);
                    });
                } else {
                    self.data.inspector_view_mode = InspectorViewMode::Editor;
                    self.inspector_editor_input.update(cx, |s, cx| {
                        s.set_placeholder("输入文本或 JSON 内容...", window, cx);
                        s.set_highlighter("text", cx);
                        s.set_value(raw, window, cx);
                    });
                }
                self.refresh_inspector_diagnostics(cx);
            }
        }
    }

    pub fn sync_inputs_for_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_sql_editor() {
            editor.focus_handle(cx).focus(window, cx);
        } else if let Some(ts) = self.active_tab().and_then(|tab| tab.table_state.as_ref()) {
            let filter = ts.where_clause.clone();
            self.where_input.update(cx, |s, cx| s.set_value(filter, window, cx));
        }
        self.sync_inspector_for_active_cell(window, cx);
    }

    pub fn activate_tab(&mut self, tab_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.data.active_tab_id == tab_id {
            return;
        }
        self.save_active_tab_inputs(cx);
        self.data.active_tab_id = tab_id;
        self.sync_inputs_for_active_tab(window, cx);
        cx.notify();
    }

    pub fn reorder_tab(&mut self, from_id: &str, to_index: usize, cx: &mut Context<Self>) {
        let Some(from_idx) = self.data.tabs.iter().position(|t| t.id == from_id) else {
            return;
        };
        if self.data.tabs.len() <= 1 {
            return;
        }
        let target_idx = to_index.min(self.data.tabs.len() - 1);
        if from_idx == target_idx {
            return;
        }
        let tab = self.data.tabs.remove(from_idx);
        self.data.tabs.insert(target_idx, tab);
        cx.notify();
    }

    pub fn close_tab(&mut self, tab_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pos) = self.data.tabs.iter().position(|t| t.id == tab_id) else {
            return;
        };
        let was_active = self.data.active_tab_id == tab_id;
        if !was_active {
            self.save_active_tab_inputs(cx);
        }
        self.data.tabs.remove(pos);
        if was_active {
            let next_id = if self.data.tabs.is_empty() {
                String::new()
            } else {
                let next_idx = pos.min(self.data.tabs.len() - 1);
                self.data.tabs[next_idx].id.clone()
            };
            self.data.active_tab_id = next_id;
            self.sync_inputs_for_active_tab(window, cx);
        }
        cx.notify();
    }

    pub fn close_other_tabs(&mut self, keep_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.save_active_tab_inputs(cx);
        self.data.tabs.retain(|t| t.id == keep_id);
        self.data.active_tab_id = self
            .data
            .tabs
            .first()
            .map(|t| t.id.clone())
            .unwrap_or_default();
        self.sync_inputs_for_active_tab(window, cx);
        cx.notify();
    }

    pub fn close_tabs_to_right(
        &mut self,
        pivot_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pos) = self.data.tabs.iter().position(|t| t.id == pivot_id) else {
            return;
        };
        self.save_active_tab_inputs(cx);
        self.data.tabs.truncate(pos + 1);
        if !self.data.tabs.iter().any(|t| t.id == self.data.active_tab_id) {
            self.data.active_tab_id = self
                .data
                .tabs
                .last()
                .map(|t| t.id.clone())
                .unwrap_or_default();
            self.sync_inputs_for_active_tab(window, cx);
        }
        cx.notify();
    }

    pub fn close_all_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.data.tabs.clear();
        self.data.active_tab_id.clear();
        self.sync_inputs_for_active_tab(window, cx);
        cx.notify();
    }

    pub fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.data.active_modal = ActiveModal::CommandPalette;
        self.data.cmd_query.clear();
        self.data.cmd_selected_idx = 0;
        self.cmd_search_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.cmd_search_input.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub fn favorite_key(&self, schema: &str, name: &str) -> String {
        let conn_id = self
            .data
            .active_conn
            .as_ref()
            .map(|c| c.id.as_str())
            .unwrap_or("");
        format!("{conn_id}::{}::{schema}::{name}", self.data.current_db)
    }

    pub fn is_favorite(&self, schema: &str, name: &str) -> bool {
        self.data
            .favorites
            .contains(&self.favorite_key(schema, name))
    }

    pub fn current_schema_favorites(&self) -> HashSet<(String, String)> {
        let mut out = HashSet::new();
        for objects in self.data.schema_objects.values() {
            for obj in objects {
                if self.is_favorite(&obj.schema, &obj.name) {
                    out.insert((obj.schema.clone(), obj.name.clone()));
                }
            }
        }
        out
    }

    pub fn toggle_favorite(&mut self, schema: &str, name: &str, cx: &mut Context<Self>) {
        let key = self.favorite_key(schema, name);
        let added = if self.data.favorites.contains(&key) {
            self.data.favorites.remove(&key);
            false
        } else {
            self.data.favorites.insert(key);
            true
        };
        save_favorites(&self.data.favorites);
        self.maybe_auto_push_sync(cx);
        self.sidebar_tree = None;
        if added {
            self.show_toast(format!("已收藏 {name}"), cx);
        } else {
            self.show_toast(format!("已取消收藏 {name}"), cx);
        }
        cx.notify();
    }

    pub fn active_driver(&self) -> DbDriver {
        self.data
            .active_conn
            .as_ref()
            .map(|c| c.driver)
            .unwrap_or(DbDriver::Postgres)
    }

    pub fn qualified_table_sql(&self, schema: &str, name: &str) -> String {
        build_qualified_table_sql(self.active_driver(), schema, name)
    }

    pub fn open_table_select_sql(
        &mut self,
        schema: &str,
        table: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let qualified = self.qualified_table_sql(schema, table);
        let limit = self.data.settings.default_limit.max(1);
        let sql = format!("SELECT *\nFROM {qualified}\nLIMIT {limit};");
        self.add_sql_tab_with_sql(sql, window, cx);
        self.run_active_sql(cx);
    }

    pub fn open_table_count_sql(
        &mut self,
        schema: &str,
        table: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let qualified = self.qualified_table_sql(schema, table);
        let sql = format!("SELECT COUNT(*) AS total_rows\nFROM {qualified};");
        self.add_sql_tab_with_sql(sql, window, cx);
        self.run_active_sql(cx);
    }

    pub fn copy_table_ddl(&mut self, schema: &str, table: &str, cx: &mut Context<Self>) {
        if let Some(ddl) = self
            .data
            .tabs
            .iter()
            .filter_map(|t| t.table_state.as_ref())
            .find(|ts| ts.schema == schema && ts.table == table)
            .and_then(|ts| ts.structure.as_ref())
            .map(|st| crate::ui::structure_view::format_ddl(&st.ddl, self.data.settings.tab_size))
        {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(ddl));
            self.show_toast(format!("已复制 {table} 的 DDL"), cx);
            return;
        }
        let Some(adapter) = self.data.adapter.clone() else {
            return;
        };
        let sch = schema.to_string();
        let tbl = table.to_string();
        let tab_size = self.data.settings.tab_size;
        let session = self.session_generation;
        let rt = self.rt.clone();
        cx.spawn(async move |this, cx| {
            let res = rt
                .spawn(async move { adapter.get_table_structure(&sch, &tbl).await })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.session_generation != session {
                    return;
                }
                match res {
                    Ok(Ok(st)) => {
                        let ddl = crate::ui::structure_view::format_ddl(&st.ddl, tab_size);
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(ddl));
                        app.show_toast(format!("已复制 {} 的 DDL", st.table), cx);
                    }
                    Ok(Err(e)) => app.show_toast(format!("获取 DDL 失败: {e}"), cx),
                    Err(e) => app.show_toast(format!("获取 DDL 失败: {e}"), cx),
                }
            });
        })
        .detach();
    }

    pub fn open_table_export(
        &mut self,
        schema: &str,
        table: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_table_tab_with_subview(schema, table, None, TableSubview::Data, window, cx);
        self.data.active_modal = ActiveModal::ExportImport { is_import: false };
        cx.notify();
    }

    pub fn open_rename_table_modal(
        &mut self,
        schema: &str,
        table: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.data.active_conn.as_ref().is_some_and(|c| c.read_only) {
            self.show_toast("当前连接为只读模式，无法重命名表", cx);
            return;
        }
        self.rename_table_input
            .update(cx, |s, cx| s.set_value(table.to_string(), window, cx));
        self.data.active_modal = ActiveModal::RenameTable {
            schema: schema.to_string(),
            table: table.to_string(),
        };
        self.rename_table_input.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub fn confirm_rename_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ActiveModal::RenameTable { schema, table } = self.data.active_modal.clone() else {
            return;
        };
        let new_name = self.rename_table_input.read(cx).value().trim().to_string();
        if new_name.is_empty() {
            self.show_toast("请输入新的表名", cx);
            return;
        }
        if new_name == table {
            self.data.active_modal = ActiveModal::None;
            cx.notify();
            return;
        }
        let Some(adapter) = self.data.adapter.clone() else {
            return;
        };
        let read_only = self.data.active_conn.as_ref().is_some_and(|c| c.read_only);
        if read_only {
            self.show_toast("当前连接为只读模式，已阻止重命名操作", cx);
            return;
        }
        let sql = build_rename_table_sql(self.active_driver(), &schema, &table, &new_name);
        self.data.active_modal = ActiveModal::None;
        self.focus_handle.focus(window, cx);
        cx.notify();

        let session = self.session_generation;
        let rt = self.rt.clone();
        let sql_run = sql.clone();
        cx.spawn(async move |this, cx| {
            let res = rt
                .spawn(async move { adapter.execute_sql(&sql_run, false).await })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.session_generation != session {
                    return;
                }
                match res {
                    Ok(Ok(_)) => {
                        let old_key = app.favorite_key(&schema, &table);
                        if app.data.favorites.remove(&old_key) {
                            let new_key = app.favorite_key(&schema, &new_name);
                            app.data.favorites.insert(new_key);
                            save_favorites(&app.data.favorites);
                        }
                        for tab in &mut app.data.tabs {
                            if let Some(ts) = tab.table_state.as_mut() {
                                if ts.schema == schema && ts.table == table {
                                    ts.table = new_name.clone();
                                    tab.title = new_name.clone();
                                }
                            }
                        }
                        let cur_schema = app.data.current_schema.clone();
                        app.refresh_schema(cur_schema, cx);
                        app.show_toast(format!("已将表 {table} 重命名为 {new_name}"), cx);
                    }
                    Ok(Err(e)) => app.show_toast(format!("重命名失败: {e}"), cx),
                    Err(e) => app.show_toast(format!("重命名失败: {e}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn open_table_danger_modal(
        &mut self,
        action: TableDangerAction,
        schema: &str,
        table: &str,
        cx: &mut Context<Self>,
    ) {
        if self.data.active_conn.as_ref().is_some_and(|c| c.read_only) {
            self.show_toast("当前连接为只读模式，无法执行写/DDL 操作", cx);
            return;
        }
        self.data.active_modal = ActiveModal::ConfirmTableAction {
            action,
            schema: schema.to_string(),
            table: table.to_string(),
        };
        cx.notify();
    }

    pub fn execute_table_danger_action(
        &mut self,
        action: TableDangerAction,
        schema: String,
        table: String,
        cx: &mut Context<Self>,
    ) {
        let Some(adapter) = self.data.adapter.clone() else {
            return;
        };
        if self.data.active_conn.as_ref().is_some_and(|c| c.read_only) {
            self.show_toast("当前连接为只读模式，已阻止该操作", cx);
            return;
        }
        let sql = build_danger_table_sql(self.active_driver(), action, &schema, &table);
        self.data.active_modal = ActiveModal::None;
        cx.notify();

        let session = self.session_generation;
        let rt = self.rt.clone();
        let this = cx.weak_entity();
        self.window_cx
            .spawn(async move |cx| {
                let res = rt
                    .spawn(async move { adapter.execute_sql(&sql, false).await })
                    .await;
                let _ = this.update_in(cx, |app, window, cx| {
                    if app.session_generation != session {
                        return;
                    }
                    match res {
                        Ok(Ok(_)) => match action {
                            TableDangerAction::Truncate => {
                                let is_active_table = app
                                    .active_tab()
                                    .and_then(|t| t.table_state.as_ref())
                                    .is_some_and(|ts| ts.schema == schema && ts.table == table);
                                if is_active_table {
                                    app.query_active_table(cx);
                                }
                                app.show_toast(format!("已清空表 {table}"), cx);
                            }
                            TableDangerAction::Drop => {
                                let key = app.favorite_key(&schema, &table);
                                if app.data.favorites.remove(&key) {
                                    save_favorites(&app.data.favorites);
                                }
                                let tab_ids: Vec<String> = app
                                    .data
                                    .tabs
                                    .iter()
                                    .filter(|t| {
                                        t.table_state.as_ref().is_some_and(|ts| {
                                            ts.schema == schema && ts.table == table
                                        })
                                    })
                                    .map(|t| t.id.clone())
                                    .collect();
                                for tid in tab_ids {
                                    app.close_tab(&tid, window, cx);
                                }
                                let cur_schema = app.data.current_schema.clone();
                                app.refresh_schema(cur_schema, cx);
                                app.show_toast(format!("已删除表 {table}"), cx);
                            }
                        },
                        Ok(Err(e)) => app.show_toast(format!("执行失败: {e}"), cx),
                        Err(e) => app.show_toast(format!("执行失败: {e}"), cx),
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    pub fn focus_sidebar_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.data.sidebar_open = true;
        self.sidebar_filter_input.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub fn open_first_filtered_object(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((schema, name)) = self.first_matching_openable_object() {
            self.open_table_tab(&schema, &name, None, window, cx);
            self.focus_handle.focus(window, cx);
        }
    }

    pub fn copy_active_cell(&mut self, cx: &mut Context<Self>) {
        let val = match self.active_tab() {
            Some(tab) if tab.kind == TabKind::Table => tab.table_state.as_ref().and_then(|ts| {
                ts.rows
                    .get(ts.selected_row)
                    .and_then(|r| r.get(ts.selected_col_idx))
                    .map(|v| v.as_deref().unwrap_or("NULL").to_string())
            }),
            Some(tab) if tab.kind == TabKind::Sql => tab.sql_state.as_ref().and_then(|ss| {
                let r = ss.selected_row?;
                let c = ss.selected_col_idx?;
                ss.result
                    .as_ref()
                    .and_then(|res| res.rows.get(r))
                    .and_then(|row| row.get(c))
                    .map(|v| v.as_deref().unwrap_or("NULL").to_string())
            }),
            _ => None,
        };
        if let Some(v) = val {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(v));
            self.show_toast("已复制单元格值", cx);
        }
    }

    pub fn move_grid_selection(
        &mut self,
        dr: isize,
        dc: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use super::data_grid::{column_width, reveal_grid_cell, ROW_NUMBER_WIDTH};
        let active_kind = self.active_tab().map(|t| t.kind);
        let mut changed = false;
        match active_kind {
            Some(TabKind::Table) => {
                if let Some(ts) = self.active_tab_mut().and_then(|t| t.table_state.as_mut()) {
                    if ts.subview != TableSubview::Data {
                        return;
                    }
                    let row_count = ts.rows.len();
                    let visible_cols: Vec<usize> = ts
                        .columns
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| !ts.hidden_cols.contains(&c.name))
                        .map(|(i, _)| i)
                        .collect();
                    if row_count > 0 && !visible_cols.is_empty() {
                        let new_r = (ts.selected_row as isize + dr)
                            .clamp(0, (row_count - 1) as isize)
                            as usize;
                        let cur_vis = visible_cols
                            .iter()
                            .position(|&ci| ci == ts.selected_col_idx)
                            .unwrap_or(0);
                        let new_vis = (cur_vis as isize + dc)
                            .clamp(0, (visible_cols.len() - 1) as isize)
                            as usize;
                        let new_c = visible_cols[new_vis];
                        if new_r != ts.selected_row || new_c != ts.selected_col_idx {
                            let left = visible_cols[..new_vis]
                                .iter()
                                .fold(ROW_NUMBER_WIDTH, |x, &i| x + column_width(ts, &ts.columns[i]));
                            reveal_grid_cell(
                                &ts.vertical_scroll, &ts.horizontal_scroll, new_r, left,
                                column_width(ts, &ts.columns[new_c]),
                            );
                            ts.selected_row = new_r;
                            ts.selected_col_idx = new_c;
                            ts.editing_cell = None;
                            changed = true;
                        }
                    }
                }
            }
            Some(TabKind::Sql) => {
                if let Some(ss) = self.active_tab_mut().and_then(|t| t.sql_state.as_mut()) {
                    if ss.result_tab != SqlResultTab::Result {
                        return;
                    }
                    if let Some(res) = &ss.result {
                        if !res.rows.is_empty() && !res.columns.is_empty() {
                            let cur_r = ss.selected_row.unwrap_or(0);
                            let cur_c = ss.selected_col_idx.unwrap_or(0);
                            let new_r = (cur_r as isize + dr)
                                .clamp(0, (res.rows.len() - 1) as isize)
                                as usize;
                            let new_c = (cur_c as isize + dc)
                                .clamp(0, (res.columns.len() - 1) as isize)
                                as usize;
                            if ss.selected_row != Some(new_r) || ss.selected_col_idx != Some(new_c) {
                                let left = (0..new_c).map(|i| ss.column_width(i)).sum();
                                reveal_grid_cell(
                                    &ss.vertical_scroll, &ss.horizontal_scroll, new_r, left,
                                    ss.column_width(new_c),
                                );
                                ss.selected_row = Some(new_r);
                                ss.selected_col_idx = Some(new_c);
                                changed = true;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        if changed {
            self.sync_inspector_for_active_cell(window, cx);
            cx.notify();
        }
    }

    fn load_connection<T: Send + 'static>(
        &mut self,
        label: String,
        work: impl std::future::Future<Output = crate::adapter::AdapterResult<T>> + Send + 'static,
        on_success: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        // The newest navigation owns the progress indicator and the workspace.
        // Dropping the previous handle also cancels its outstanding catalog reads.
        self.pending_connection.take();
        let task = self.rt.spawn(work);
        let id = task.id();
        let bar = cx.new(|_| ConnectionProgressBar {
            label: format!("正在{label}…").into(),
        });
        self.pending_connection = Some(ConnectionLoad {
            label,
            bar,
            task: task.abort_handle(),
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                let Some(load) = app.pending_connection.take_if(|load| load.task.id() == id) else {
                    return;
                };
                match result {
                    Ok(Ok(value)) => on_success(app, value, cx),
                    Ok(Err(e)) => app.show_toast(format!("{}失败：{e}", load.label), cx),
                    Err(e) => app.show_toast(format!("{}失败：{e}", load.label), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn connect_with_config(&mut self, cfg: ConnectionConfig, cx: &mut Context<Self>) {
        self.open_database(cfg, false, cx);
    }

    fn open_database(&mut self, cfg: ConnectionConfig, switching: bool, cx: &mut Context<Self>) {
        if !self.data.staged_changes.is_empty() {
            self.show_toast("请先提交或丢弃当前连接的暂存修改", cx);
            return;
        }
        let label = if switching {
            format!("切换数据库 {}", cfg.database)
        } else {
            format!("连接 {}", cfg.name)
        };
        let work = async move {
            let adapter = create_adapter(cfg)?;
            let info = if switching {
                None
            } else {
                Some(adapter.test_connection().await?)
            };
            let has_selected_db = !adapter.config().database.trim().is_empty();
            // MySQL's schema list is its database list. A database switch on the
            // same PostgreSQL server can reuse the already displayed database list.
            let databases = if !switching && (adapter.driver() != DbDriver::Mysql || !has_selected_db) {
                adapter.list_databases().await.unwrap_or_default()
            } else {
                Vec::new()
            };
            let objects = if has_selected_db {
                let schema = if adapter.driver() == DbDriver::Mysql {
                    adapter.config().database.as_str()
                } else {
                    ""
                };
                adapter.list_schema_objects(schema).await?
            } else {
                SchemaObjects {
                    schemas: Vec::new(),
                    groups: Default::default(),
                }
            };
            Ok((adapter, info, databases, objects))
        };
        self.load_connection(
            label,
            work,
            move |app, (adapter, info, databases, objects), cx| {
                if !app.data.staged_changes.is_empty() {
                    app.show_toast("请先提交或丢弃当前连接的暂存修改", cx);
                    return;
                }
                app.session_generation += 1;
                let cfg = adapter.config();
                let has_selected_db = !cfg.database.trim().is_empty();
                if cfg.driver == DbDriver::Mysql && has_selected_db {
                    app.data.databases = objects.schemas.clone();
                } else if !switching {
                    app.data.databases = databases;
                    if app.data.databases.is_empty() && has_selected_db {
                        app.data.databases.push(cfg.database.clone());
                    }
                }
                app.data.current_db = if has_selected_db {
                    cfg.database.clone()
                } else {
                    String::new()
                };
                let message = if let Some(info) = info {
                    app.data.rtt_ms = info.rtt_ms;
                    app.data.server_version = info.server_version;
                    format!("已连接至 {} ({}ms)", cfg.name, info.rtt_ms)
                } else {
                    format!("已切换数据库：{}", app.data.current_db)
                };
                if switching {
                    // Keep SQL drafts. Results and editable table views belong to the old database.
                    app.data.tabs.retain(|tab| tab.kind != TabKind::Table);
                    for sql in app
                        .data
                        .tabs
                        .iter_mut()
                        .filter_map(|tab| tab.sql_state.as_mut())
                    {
                        sql.result = None;
                        sql.explainable_sql = None;
                        sql.explain = None;
                        sql.messages.clear();
                        sql.selected_row = None;
                        sql.selected_col_idx = None;
                        sql.is_loading = false;
                    }
                } else {
                    app.data.tabs.clear();
                }
                if app.active_tab().is_none() {
                    app.data.active_tab_id.clear();
                }
                app.data.monitor_metrics = None;
                app.data.ctx_menu.open = false;
                app.data.sidebar_ctx_menu.open = false;
                app.data.tab_ctx_menu.open = false;
                app.data.drawer_open = false;
                app.data.is_connected = true;
                app.data.active_conn = Some(cfg.clone());
                app.data.active_modal = ActiveModal::None;
                app.data.new_conn.editing_id = None;
                app.data.new_conn.return_to_manager = false;
                let schema = if !has_selected_db {
                    String::new()
                } else if cfg.driver == DbDriver::Mysql {
                    cfg.database.clone()
                } else {
                    objects.schemas.first().cloned().unwrap_or_default()
                };
                app.data.adapter = Some(adapter);
                app.set_schema(schema, objects, true);
                if app
                    .active_tab()
                    .is_some_and(|tab| tab.kind == TabKind::Monitor)
                {
                    app.refresh_monitor(cx);
                }
                app.show_toast(message, cx);
            },
            cx,
        );
    }

    fn set_schema(&mut self, schema: String, objects: SchemaObjects, reset_columns: bool) {
        self.data.current_schema = schema;
        self.data.schemas = objects.schemas;
        self.sidebar_tree = None;
        self.data.schema_objects = Arc::new(objects.groups);
        self.data.db_dropdown_open = false;
        self.data.schema_dropdown_open = false;
        if let Ok(mut cat) = self.sql_lsp_catalog.write() {
            if reset_columns {
                cat.columns_by_table.clear();
                for table in self.data.tabs.iter().filter_map(|t| t.table_state.as_ref()) {
                    if table.schema == self.data.current_schema && table.structure.is_some() {
                        cat.update_table_columns(&table.table, Arc::clone(&table.columns));
                    }
                }
            }
            cat.update_schema_objects(
                self.data.active_conn.as_ref().unwrap().driver,
                self.data.current_db.clone(),
                self.data.current_schema.clone(),
                self.data.databases.clone(),
                self.data.schemas.clone(),
                Arc::clone(&self.data.schema_objects),
            );
        }
    }

    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.pending_connection.take();
        self.session_generation += 1;
        self.data.is_connected = false;
        self.data.active_conn = None;
        self.data.adapter = None;
        self.data.tabs.clear();
        self.data.active_tab_id.clear();
        self.data.staged_changes.clear();
        self.data.active_modal = ActiveModal::None;
        self.data.monitor_metrics = None;
        self.data.current_db.clear();
        self.data.databases.clear();
        self.data.current_schema.clear();
        self.data.schemas.clear();
        self.sidebar_tree = None;
        self.data.schema_objects = Arc::default();
        if let Ok(mut cat) = self.sql_lsp_catalog.write() {
            *cat = SqlLspCatalog::default();
        }
        self.show_toast("已断开连接", cx);
    }

    pub fn switch_database(&mut self, db: String, cx: &mut Context<Self>) {
        self.data.db_dropdown_open = false;
        self.data.schema_dropdown_open = false;
        let Some(mut cfg) = self.data.active_conn.clone() else {
            return;
        };
        if cfg.database == db {
            self.pending_connection.take();
            cx.notify();
            return;
        }
        cfg.database = db;
        self.open_database(cfg, true, cx);
    }

    pub fn refresh_schema(&mut self, schema: String, cx: &mut Context<Self>) {
        let Some(adapter) = self.data.adapter.clone() else {
            return;
        };
        self.data.schema_dropdown_open = false;
        self.data.db_dropdown_open = false;
        if self.data.current_db.is_empty() {
            self.load_connection(
                "刷新数据库列表".to_string(),
                async move { adapter.list_databases().await },
                |app, databases, cx| {
                    app.data.databases = databases;
                    app.show_toast("数据库列表已刷新", cx);
                },
                cx,
            );
            return;
        }
        let label = if schema == self.data.current_schema {
            format!("刷新 {schema} 的元数据")
        } else {
            format!("切换 Schema 至 {schema}")
        };
        self.load_connection(
            label,
            async move {
                let objects = adapter.list_schema_objects(&schema).await?;
                Ok((schema, objects))
            },
            |app, (schema, objects), cx| {
                let changed = app.data.current_schema != schema;
                app.set_schema(schema, objects, changed);
                app.show_toast("元数据已刷新", cx);
            },
            cx,
        );
    }

    pub fn open_table_tab(
        &mut self,
        schema: &str,
        table: &str,
        where_filter: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_table_tab_with_subview(
            schema,
            table,
            where_filter,
            TableSubview::Data,
            window,
            cx,
        );
    }

    pub fn open_table_tab_with_subview(
        &mut self,
        schema: &str,
        table: &str,
        where_filter: Option<String>,
        subview: TableSubview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_table_tab_with_target(
            schema,
            table,
            where_filter,
            subview,
            StructTab::Columns,
            window,
            cx,
        );
    }

    pub fn open_table_ddl_tab(
        &mut self,
        schema: &str,
        table: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_table_tab_with_target(
            schema,
            table,
            None,
            TableSubview::Structure,
            StructTab::Ddl,
            window,
            cx,
        );
    }

    fn open_table_tab_with_target(
        &mut self,
        schema: &str,
        table: &str,
        where_filter: Option<String>,
        subview: TableSubview,
        struct_tab: StructTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_active_tab_inputs(cx);
        if let Some(existing_id) = self
            .data
            .tabs
            .iter()
            .find(|t| {
                t.kind == TabKind::Table
                    && t.table_state
                        .as_ref()
                        .map(|ts| ts.schema == schema && ts.table == table)
                        .unwrap_or(false)
            })
            .map(|t| t.id.clone())
        {
            self.data.active_tab_id = existing_id;
            let mut should_query = false;
            if let Some(ts) = self.active_tab_mut().and_then(|t| t.table_state.as_mut()) {
                ts.subview = subview;
                if subview == TableSubview::Structure {
                    ts.struct_tab = struct_tab;
                }
                if let Some(w) = where_filter {
                    ts.applied_where_clause.clone_from(&w);
                    ts.where_clause = w;
                    ts.page = 0;
                    ts.vertical_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
                    should_query = true;
                }
            }
            self.sync_inputs_for_active_tab(window, cx);
            if should_query {
                self.query_active_table(cx);
            } else {
                cx.notify();
            }
            return;
        }

        let tab_id = format!("tab-{schema}-{table}-{}", uuid::Uuid::new_v4().simple());
        let where_clause = where_filter.unwrap_or_default();
        let show_lines = self.data.settings.show_line_numbers;
        let ddl_editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .line_number(show_lines)
                .folding(true)
                .soft_wrap(false)
        });
        let tab = TabItem {
            id: tab_id.clone(),
            title: table.to_string(),
            kind: TabKind::Table,
            table_state: Some(TableTabState {
                schema: schema.to_string(),
                table: table.to_string(),
                subview,
                struct_tab,
                columns: Arc::default(),
                rows: Vec::new(),
                structure: None,
                ddl_editor,
                ddl_formatted: true,
                ddl_soft_wrap: false,
                query_generation: 0,
                total_estimate: 0,
                has_next_page: false,
                page_error: None,
                elapsed_ms: 0.0,
                applied_where_clause: where_clause.clone(),
                where_clause,
                sort_col: None,
                sort_asc: true,
                limit: self.data.settings.default_limit.max(1),
                page: 0,
                hidden_cols: HashSet::new(),
                selected_row: 0,
                selected_col_idx: 0,
                checked_rows: HashSet::new(),
                added_rows: HashSet::new(),
                deleted_rows: HashSet::new(),
                editing_cell: None,
                is_loading: true,
                col_widths: std::collections::HashMap::new(),
                vertical_scroll: gpui::UniformListScrollHandle::new(),
                horizontal_scroll: gpui::ScrollHandle::new(),
            }),
            sql_state: None,
        };
        self.data.tabs.push(tab);
        self.data.active_tab_id = tab_id;
        self.sync_inputs_for_active_tab(window, cx);
        self.query_active_table(cx);
    }

    pub(super) fn apply_table_filter(&mut self, filter: String, cx: &mut Context<Self>) {
        if let Some(ts) = self.active_tab_mut().and_then(|tab| tab.table_state.as_mut()) {
            ts.where_clause.clone_from(&filter);
            ts.applied_where_clause = filter;
            ts.page = 0;
            ts.vertical_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
            self.query_active_table(cx);
        }
    }

    pub fn query_active_table(&mut self, cx: &mut Context<Self>) {
        self.load_table_page(false, cx);
    }

    pub fn reload_active_table(&mut self, cx: &mut Context<Self>) {
        self.load_table_page(true, cx);
    }

    fn load_table_page(&mut self, reload_structure: bool, cx: &mut Context<Self>) {
        let Some(adapter) = self.data.adapter.clone() else {
            return;
        };
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let Some(ts) = tab.table_state.as_mut() else {
            return;
        };
        ts.is_loading = true;
        ts.page_error = None;
        ts.query_generation += 1;
        let generation = ts.query_generation;
        let tab_id = tab.id.clone();
        let cached = if reload_structure {
            None
        } else {
            ts.structure.clone()
        };
        let query = TableDataQuery {
            schema: ts.schema.clone(),
            table: ts.table.clone(),
            where_clause: (!ts.applied_where_clause.trim().is_empty())
                .then(|| ts.applied_where_clause.clone()),
            sort_col: ts.sort_col.clone(),
            sort_asc: ts.sort_asc,
            limit: ts.limit,
            offset: ts.page.saturating_mul(ts.limit),
        };
        let task = self.rt.spawn(async move {
            let started = std::time::Instant::now();
            let structure = match cached {
                Some(structure) => structure,
                None => Arc::new(
                    adapter
                        .get_table_structure(&query.schema, &query.table)
                        .await?,
                ),
            };
            let mut page = adapter.query_table_page(&query, &structure.columns).await?;
            page.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            Ok::<_, crate::adapter::AdapterError>((page, structure))
        });
        cx.notify();
        let this = cx.weak_entity();
        self.window_cx.spawn(async move |cx| {
            let result = task.await;
            let _ = this.update_in(cx, |app, window, cx| {
                let Some(ts) = app
                    .data
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
                    .and_then(|tab| tab.table_state.as_mut())
                else {
                    return;
                };
                if ts.query_generation != generation {
                    return;
                }
                ts.is_loading = false;
                match result {
                    Ok(Ok((page, structure))) => {
                        if !ts
                            .structure
                            .as_ref()
                            .is_some_and(|old| Arc::ptr_eq(old, &structure))
                        {
                            ts.columns = Arc::clone(&structure.columns);
                            if ts.schema == app.data.current_schema {
                                if let Ok(mut cat) = app.sql_lsp_catalog.write() {
                                    cat.update_table_columns(&ts.table, Arc::clone(&ts.columns));
                                }
                            }
                            let ddl_text = if ts.ddl_formatted {
                                crate::ui::structure_view::format_ddl(
                                    &structure.ddl,
                                    app.data.settings.tab_size,
                                )
                            } else {
                                structure.ddl.clone()
                            };
                            ts.ddl_editor.update(cx, |ed, cx| {
                                ed.set_value(ddl_text, window, cx);
                            });
                            ts.structure = Some(structure);
                        }
                        ts.rows = page.rows;
                        ts.has_next_page = page.has_more;
                        ts.checked_rows.clear();
                        ts.editing_cell = None;
                        ts.total_estimate = page.total_estimate;
                        ts.elapsed_ms = page.elapsed_ms;
                        ts.selected_row = ts.selected_row.min(ts.rows.len().saturating_sub(1));
                        ts.selected_col_idx =
                            ts.selected_col_idx.min(ts.columns.len().saturating_sub(1));
                        if app.data.active_tab_id == tab_id {
                            app.sync_inspector_for_active_cell(window, cx);
                        }
                    }
                    Ok(Err(e)) => ts.page_error = Some(e.to_string()),
                    Err(e) => ts.page_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn add_sql_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_sql_tab_with_sql(String::new(), window, cx);
    }

    pub fn add_sql_tab_with_sql(
        &mut self,
        initial_sql: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_active_tab_inputs(cx);
        let count = self
            .data
            .tabs
            .iter()
            .filter(|t| t.kind == TabKind::Sql)
            .count()
            + 1;
        let id = format!("tab-sql-{}", uuid::Uuid::new_v4().simple());
        let provider = Rc::new(SqlLspProvider::new(Arc::clone(&self.sql_lsp_catalog)));
        let show_lines = self.data.settings.show_line_numbers;
        let editor = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language("sql")
                .line_number(show_lines)
                .soft_wrap(false)
                .default_value(initial_sql);
            state.lsp_mut().completion_provider = Some(provider.clone());
            state.lsp_mut().hover_provider = Some(provider);
            state
        });
        self.data.tabs.push(TabItem {
            id: id.clone(),
            title: format!("Query {count}"),
            kind: TabKind::Sql,
            table_state: None,
            sql_state: Some(SqlTabState {
                editor,
                result_tab: SqlResultTab::Result,
                result: None,
                explainable_sql: None,
                explain: None,
                messages: vec!["Ready.".to_string()],
                show_snippets_menu: false,
                is_loading: false,
                selected_row: None,
                selected_col_idx: None,
                col_widths: std::collections::HashMap::new(),
                vertical_scroll: gpui::UniformListScrollHandle::new(),
                horizontal_scroll: gpui::ScrollHandle::new(),
            }),
        });
        self.data.active_tab_id = id;
        self.sync_inputs_for_active_tab(window, cx);
        cx.notify();
    }

    pub fn open_monitor_tab(&mut self, cx: &mut Context<Self>) {
        if let Some(existing) = self.data.tabs.iter().find(|t| t.kind == TabKind::Monitor) {
            self.data.active_tab_id = existing.id.clone();
            self.refresh_monitor(cx);
            return;
        }
        let id = "tab-monitor".to_string();
        self.data.tabs.push(TabItem {
            id: id.clone(),
            title: "Server Monitor".to_string(),
            kind: TabKind::Monitor,
            table_state: None,
            sql_state: None,
        });
        self.data.active_tab_id = id;
        self.refresh_monitor(cx);
    }

    pub fn refresh_monitor(&mut self, cx: &mut Context<Self>) {
        let Some(adapter) = self.data.adapter.clone() else {
            return;
        };
        let session = self.session_generation;
        let rt = self.rt.clone();
        cx.spawn(async move |this, cx| {
            let res = rt.spawn(async move { adapter.get_server_metrics().await }).await;
            let _ = this.update(cx, |app, cx| {
                if app.session_generation != session {
                    return;
                }
                match res {
                    Ok(Ok(metrics)) => {
                        app.data.monitor_metrics = Some(metrics);
                    }
                    Ok(Err(e)) => app.show_toast(format!("监控刷新失败: {e}"), cx),
                    Err(e) => app.show_toast(format!("错误: {e}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn cancel_session(&mut self, pid: i32, terminate: bool, cx: &mut Context<Self>) {
        let Some(adapter) = self.data.adapter.clone() else {
            return;
        };
        let session = self.session_generation;
        let rt = self.rt.clone();
        cx.spawn(async move |this, cx| {
            let res = rt
                .spawn(async move { adapter.cancel_session(pid, terminate).await })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.session_generation != session {
                    return;
                }
                match res {
                    Ok(Ok(_)) => {
                        app.show_toast(format!("已终止会话 {pid}"), cx);
                        app.refresh_monitor(cx);
                    }
                    Ok(Err(e)) => app.show_toast(format!("终止失败: {e}"), cx),
                    Err(e) => app.show_toast(format!("错误: {e}"), cx),
                }
            });
        })
        .detach();
    }

    pub fn select_sql_result_tab(&mut self, target: SqlResultTab, cx: &mut Context<Self>) {
        let Some(ss) = self.active_tab_mut().and_then(|tab| tab.sql_state.as_mut()) else {
            return;
        };
        ss.result_tab = target;
        if target == SqlResultTab::Explain && ss.explain.is_none() && !ss.is_loading {
            if let Some(sql) = ss.explainable_sql.clone() {
                self.request_explain(sql, cx);
            }
        }
        cx.notify();
    }

    pub fn explain_active_sql(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_sql_editor() {
            self.request_explain(editor.read(cx).text().to_string().into(), cx);
        }
    }

    fn request_explain(&mut self, sql: Arc<str>, cx: &mut Context<Self>) {
        let Some(adapter) = self.data.adapter.clone() else {
            self.show_toast("请先连接数据库", cx);
            return;
        };
        let tab_id = self.data.active_tab_id.clone();
        let Some(ss) = self.active_tab_mut().and_then(|tab| tab.sql_state.as_mut()) else {
            return;
        };
        if ss.is_loading {
            return;
        }
        ss.result_tab = SqlResultTab::Explain;
        ss.is_loading = true;
        cx.notify();
        let session = self.session_generation;
        let rt = self.rt.clone();
        cx.spawn(async move |this, cx| {
            let res = rt
                .spawn(async move { adapter.explain_sql(&sql).await })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.session_generation != session {
                    return;
                }
                if let Some(t) = app.data.tabs.iter_mut().find(|t| t.id == tab_id) {
                    if let Some(ss) = t.sql_state.as_mut() {
                        ss.is_loading = false;
                    }
                }
                match res {
                    Ok(Ok(plan)) => {
                        if let Some(t) = app.data.tabs.iter_mut().find(|t| t.id == tab_id) {
                            if let Some(ss) = t.sql_state.as_mut() {
                                ss.explain = Some(plan);
                            }
                        }
                        app.show_toast("执行计划已生成", cx);
                    }
                    Ok(Err(e)) => app.show_toast(format!("EXPLAIN 失败: {e}"), cx),
                    Err(e) => app.show_toast(format!("错误: {e}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn run_active_sql(&mut self, cx: &mut Context<Self>) {
        let Some(adapter) = self.data.adapter.clone() else {
            self.show_toast("请先连接数据库", cx);
            return;
        };
        let tab_id = self.data.active_tab_id.clone();
        let Some(ss) = self.active_tab_mut().and_then(|tab| tab.sql_state.as_mut()) else { return };
        if ss.is_loading {
            return;
        }
        let sql = ss.editor.read(cx).text().to_string();
        ss.is_loading = true;
        cx.notify();
        let read_only = self.data.active_conn.as_ref().map(|c| c.read_only).unwrap_or(false);
        let session = self.session_generation;
        let rt = self.rt.clone();

        let this = cx.weak_entity();
        self.window_cx.spawn(async move |cx| {
            let res = rt.spawn(async move {
                let result = adapter.execute_sql(&sql, read_only).await?;
                let explainable = crate::adapter::postgres::is_read_only_sql(&sql);
                Ok::<_, crate::adapter::AdapterError>((result, Arc::<str>::from(sql), explainable))
            }).await;

            let _ = this.update_in(cx, |app, window, cx| {
                if app.session_generation != session {
                    return;
                }
                if let Some(t) = app.data.tabs.iter_mut().find(|t| t.id == tab_id) {
                    if let Some(ss) = t.sql_state.as_mut() {
                        ss.is_loading = false;
                    }
                }
                match res {
                    Ok(Ok((qres, sql, explainable))) => {
                        let time_str = chrono::Local::now().format("%H:%M").to_string();
                        let ms_str = format!("{:.1} ms", qres.elapsed_ms);
                        let explainable_sql = explainable.then(|| Arc::clone(&sql));
                        app.data.history.insert(
                            0,
                            QueryHistoryEntry {
                                time: time_str,
                                elapsed_ms: ms_str.clone(),
                                sql,
                            },
                        );
                        app.data.history.truncate(50);
                        save_history(&app.data.history);

                        if let Some(t) = app.data.tabs.iter_mut().find(|t| t.id == tab_id) {
                            if let Some(ss) = t.sql_state.as_mut() {
                                ss.messages.push(qres.message.clone());
                                ss.selected_row = if qres.rows.is_empty() { None } else { Some(0) };
                                ss.selected_col_idx =
                                    if qres.columns.is_empty() { None } else { Some(0) };
                                ss.result = Some(qres);
                                ss.explain = None;
                                ss.explainable_sql = explainable_sql;
                                ss.result_tab = SqlResultTab::Result;
                            }
                        }
                        if app.data.active_tab_id == tab_id {
                            app.sync_inspector_for_active_cell(window, cx);
                        }
                        app.show_toast(format!("查询完成 ({ms_str})"), cx);
                    }
                    Ok(Err(e)) => {
                        if let Some(t) = app.data.tabs.iter_mut().find(|t| t.id == tab_id) {
                            if let Some(ss) = t.sql_state.as_mut() {
                                ss.messages.push(format!("ERROR: {e}"));
                                ss.result_tab = SqlResultTab::Messages;
                            }
                        }
                        app.show_toast(format!("SQL 执行失败: {e}"), cx);
                    }
                    Err(e) => app.show_toast(format!("错误: {e}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn stage_cell_update(
        &mut self,
        row_idx: usize,
        col_idx: usize,
        new_val: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let Some(ts) = tab.table_state.as_mut() else {
            return;
        };
        if row_idx >= ts.rows.len() || col_idx >= ts.columns.len() {
            return;
        }
        let col_name = ts.columns[col_idx].name.clone();
        let old_val = ts.rows[row_idx][col_idx].clone();
        if old_val == new_val {
            return;
        }
        let pk_idx = ts.columns.iter().position(|c| c.is_pk).unwrap_or(0);
        let pk_col = ts.columns[pk_idx].name.clone();
        let pk_val = ts.rows[row_idx][pk_idx].clone();
        let schema = ts.schema.clone();
        let table = ts.table.clone();

        ts.rows[row_idx][col_idx] = new_val.clone();

        if let Some(existing) = self.data.staged_changes.iter_mut().find(|s| {
            s.schema == schema
                && s.table == table
                && s.row_idx == row_idx
                && matches!(&s.kind, StagedChangeKind::Update { col, .. } if col == &col_name)
        }) {
            if let StagedChangeKind::Update {
                old_val: orig,
                new_val: cur_new,
                ..
            } = &mut existing.kind
            {
                if *orig == new_val {
                    self.data.staged_changes.retain(|s| {
                        !(s.schema == schema
                            && s.table == table
                            && s.row_idx == row_idx
                            && matches!(&s.kind, StagedChangeKind::Update { col, .. } if col == &col_name))
                    });
                } else {
                    *cur_new = new_val;
                }
            }
        } else {
            self.data.staged_changes.push(StagedChange {
                schema,
                table,
                row_idx,
                pk_col,
                pk_val,
                kind: StagedChangeKind::Update {
                    col: col_name,
                    old_val,
                    new_val,
                },
            });
        }
        cx.notify();
    }

    pub fn commit_staged_changes(&mut self, cx: &mut Context<Self>) {
        let Some(adapter) = self.data.adapter.clone() else {
            return;
        };
        let changes = self.data.staged_changes.clone();
        if changes.is_empty() {
            return;
        }
        let session = self.session_generation;
        let rt = self.rt.clone();
        cx.spawn(async move |this, cx| {
            let res = rt
                .spawn(async move { adapter.commit_staged_changes(&changes).await })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.session_generation != session {
                    return;
                }
                match res {
                    Ok(Ok(n)) => {
                        app.data.staged_changes.clear();
                        if let Some(ts) = app.active_tab_mut().and_then(|t| t.table_state.as_mut())
                        {
                            ts.added_rows.clear();
                            ts.deleted_rows.clear();
                        }
                        app.data.active_modal = ActiveModal::None;
                        app.show_toast(format!("✓ 事务 COMMIT 成功 ({n} 行受影响)"), cx);
                        app.query_active_table(cx);
                    }
                    Ok(Err(e)) => app.show_toast(format!("提交失败: {e}"), cx),
                    Err(e) => app.show_toast(format!("错误: {e}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn discard_staged_changes(&mut self, cx: &mut Context<Self>) {
        self.data.staged_changes.clear();
        if let Some(ts) = self.active_tab_mut().and_then(|t| t.table_state.as_mut()) {
            ts.added_rows.clear();
            ts.deleted_rows.clear();
        }
        self.query_active_table(cx);
        self.show_toast("已丢弃暂存修改", cx);
    }

    pub fn sync_new_conn_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let nc = &self.data.new_conn;
        self.nc_host_input
            .update(cx, |s, cx| s.set_value(nc.host.clone(), window, cx));
        self.nc_port_input
            .update(cx, |s, cx| s.set_value(nc.port.clone(), window, cx));
        self.nc_db_input
            .update(cx, |s, cx| s.set_value(nc.database.clone(), window, cx));
        self.nc_user_input
            .update(cx, |s, cx| s.set_value(nc.username.clone(), window, cx));
        self.nc_pass_input
            .update(cx, |s, cx| s.set_value(nc.password.clone(), window, cx));
        self.nc_name_input
            .update(cx, |s, cx| s.set_value(nc.name.clone(), window, cx));
        self.nc_tag_input
            .update(cx, |s, cx| s.set_value(nc.tag.clone(), window, cx));
    }

    pub fn sync_new_conn_ssh_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let nc = &self.data.new_conn;
        self.nc_ssh_host_input
            .update(cx, |s, cx| s.set_value(nc.ssh_host.clone(), window, cx));
        self.nc_ssh_port_input
            .update(cx, |s, cx| s.set_value(nc.ssh_port.clone(), window, cx));
        self.nc_ssh_user_input
            .update(cx, |s, cx| s.set_value(nc.ssh_user.clone(), window, cx));
        self.nc_ssh_key_input
            .update(cx, |s, cx| s.set_value(nc.ssh_key_path.clone(), window, cx));
    }

    pub fn open_new_connection_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.data.new_conn = NewConnForm::default();
        self.data.open_dropdown = None;
        self.nc_uri_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.sync_new_conn_inputs(window, cx);
        self.sync_new_conn_ssh_inputs(window, cx);
        self.data.active_modal = ActiveModal::NewConnection { step: 1 };
        cx.notify();
    }

    pub fn open_edit_connection_modal(
        &mut self,
        conn: ConnectionConfig,
        return_to_manager: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let defaults = NewConnForm::default();
        let (ssh_enabled, ssh_host, ssh_port, ssh_user, ssh_auth, ssh_key_path) =
            if let Some(ssh) = conn.ssh {
                (
                    ssh.enabled,
                    if ssh.host.is_empty() {
                        defaults.ssh_host
                    } else {
                        ssh.host
                    },
                    if ssh.port == 0 {
                        defaults.ssh_port
                    } else {
                        ssh.port.to_string()
                    },
                    if ssh.username.is_empty() {
                        defaults.ssh_user
                    } else {
                        ssh.username
                    },
                    if ssh.auth_type.is_empty() {
                        defaults.ssh_auth
                    } else {
                        ssh.auth_type
                    },
                    if ssh.private_key_path.is_empty() {
                        defaults.ssh_key_path
                    } else {
                        ssh.private_key_path
                    },
                )
            } else {
                (
                    false,
                    defaults.ssh_host,
                    defaults.ssh_port,
                    defaults.ssh_user,
                    defaults.ssh_auth,
                    defaults.ssh_key_path,
                )
            };

        self.data.new_conn = NewConnForm {
            editing_id: Some(conn.id),
            return_to_manager,
            driver: conn.driver,
            uri: String::new(),
            name: conn.name,
            tag: conn.label,
            color: conn.color,
            host: conn.host,
            port: conn.port.to_string(),
            database: conn.database,
            username: conn.username,
            password: conn.password,
            sqlite_path: conn.file_path.unwrap_or(defaults.sqlite_path),
            ssl_mode: conn.ssl_mode,
            ssh_enabled,
            ssh_host,
            ssh_port,
            ssh_user,
            ssh_auth,
            ssh_key_path,
            read_only: conn.read_only,
            test_status: None,
        };
        self.data.open_dropdown = None;
        self.nc_uri_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.sync_new_conn_inputs(window, cx);
        self.sync_new_conn_ssh_inputs(window, cx);
        self.data.active_modal = ActiveModal::NewConnection { step: 2 };
        cx.notify();
    }

    pub fn save_conn_from_form(&mut self, connect_after: bool, cx: &mut Context<Self>) {
        let cfg = self.build_conn_from_form(cx);
        if let Some(edit_id) = self.data.new_conn.editing_id.clone() {
            if let Some(pos) = self.data.connections.iter().position(|c| c.id == edit_id) {
                self.data.connections[pos] = cfg.clone();
            } else {
                self.data.connections.insert(0, cfg.clone());
            }
        } else {
            self.data.connections.insert(0, cfg.clone());
        }
        save_connections(&self.data.connections);
        self.maybe_auto_push_sync(cx);

        if connect_after {
            self.data.new_conn.editing_id = Some(cfg.id.clone());
            self.data.open_dropdown = None;
            self.connect_with_config(cfg, cx);
            return;
        }

        let mut needs_reconnect_hint = false;
        if let Some(active) = self.data.active_conn.as_mut() {
            if active.id == cfg.id {
                let conn_params_changed = active.driver != cfg.driver
                    || active.host != cfg.host
                    || active.port != cfg.port
                    || active.username != cfg.username
                    || active.password != cfg.password
                    || active.ssl_mode != cfg.ssl_mode
                    || active.file_path != cfg.file_path
                    || active.ssh != cfg.ssh;
                active.name = cfg.name.clone();
                active.env = cfg.env.clone();
                active.label = cfg.label.clone();
                active.color = cfg.color.clone();
                active.read_only = cfg.read_only;
                needs_reconnect_hint = conn_params_changed;
            }
        }

        let return_to_manager = self.data.new_conn.return_to_manager;
        self.data.new_conn.editing_id = None;
        self.data.new_conn.return_to_manager = false;
        self.data.open_dropdown = None;
        self.data.active_modal = if return_to_manager {
            ActiveModal::ConnectionManager
        } else {
            ActiveModal::None
        };
        if needs_reconnect_hint {
            self.show_toast("连接配置已保存（重新连接后生效）", cx);
        } else {
            self.show_toast("连接配置已保存", cx);
        }
    }

    pub fn build_conn_from_form(&self, cx: &gpui::App) -> ConnectionConfig {
        let nc = &self.data.new_conn;
        let host = self.nc_host_input.read(cx).value().to_string();
        let port: u16 = self
            .nc_port_input
            .read(cx)
            .value()
            .trim()
            .parse()
            .unwrap_or(nc.driver.default_port());
        let database = self.nc_db_input.read(cx).value().to_string();
        let username = self.nc_user_input.read(cx).value().to_string();
        let password = self.nc_pass_input.read(cx).value().to_string();
        let name = self.nc_name_input.read(cx).value().to_string();
        let tag = self.nc_tag_input.read(cx).value().to_string();

        let existing_ssh = nc.editing_id.as_deref().and_then(|id| {
            self.data
                .connections
                .iter()
                .find(|c| c.id == id)
                .and_then(|c| c.ssh.as_ref())
        });

        let ssh = if nc.ssh_enabled {
            Some(SshTunnelConfig {
                enabled: true,
                host: self.nc_ssh_host_input.read(cx).value().to_string(),
                port: self
                    .nc_ssh_port_input
                    .read(cx)
                    .value()
                    .trim()
                    .parse()
                    .unwrap_or(22),
                username: self.nc_ssh_user_input.read(cx).value().to_string(),
                auth_type: nc.ssh_auth.clone(),
                private_key_path: self.nc_ssh_key_input.read(cx).value().to_string(),
                passphrase: existing_ssh.and_then(|s| s.passphrase.clone()),
                password: existing_ssh.and_then(|s| s.password.clone()),
            })
        } else {
            None
        };

        let env = if tag.contains("生产") || tag.eq_ignore_ascii_case("prod") {
            "prod"
        } else if tag.contains("预发") || tag.eq_ignore_ascii_case("staging") {
            "staging"
        } else if tag.contains("本地") || tag.eq_ignore_ascii_case("local") {
            "local"
        } else {
            "custom"
        };

        ConnectionConfig {
            id: nc
                .editing_id
                .clone()
                .unwrap_or_else(|| format!("conn-{}", uuid::Uuid::new_v4().simple())),
            driver: nc.driver,
            name: if name.trim().is_empty() {
                format!("new_{}_conn", nc.driver.display_name().to_lowercase())
            } else {
                name
            },
            env: env.to_string(),
            label: if tag.trim().is_empty() {
                "自定义".to_string()
            } else {
                tag
            },
            color: nc.color.clone(),
            host: if host.trim().is_empty() {
                "127.0.0.1".to_string()
            } else {
                host
            },
            port,
            database: database.trim().to_string(),
            username: if username.trim().is_empty() {
                nc.driver.default_user().to_string()
            } else {
                username
            },
            password,
            file_path: if nc.driver.is_file_based() {
                Some(nc.sqlite_path.clone())
            } else {
                None
            },
            ssl_mode: nc.ssl_mode,
            ssh,
            read_only: nc.read_only,
        }
    }

    pub fn test_new_conn(&mut self, cx: &mut Context<Self>) {
        let cfg = self.build_conn_from_form(cx);
        self.load_connection(
            format!("测试连接 {}", cfg.name),
            async move { create_adapter(cfg)?.test_connection().await },
            |app, info, cx| {
                app.show_toast(
                    format!(
                        "✓ 连接测试成功 ({} · {}ms)",
                        info.server_version, info.rtt_ms
                    ),
                    cx,
                );
            },
            cx,
        );
    }

    pub fn build_sync_config_from_inputs(&self, cx: &gpui::App) -> SyncConfig {
        let mut cfg = self.data.sync_config.clone();
        cfg.s3.endpoint = self.sync_s3_endpoint_input.read(cx).value().trim().to_string();
        cfg.s3.region = {
            let r = self.sync_s3_region_input.read(cx).value().trim().to_string();
            if r.is_empty() { "us-east-1".to_string() } else { r }
        };
        cfg.s3.bucket = self.sync_s3_bucket_input.read(cx).value().trim().to_string();
        cfg.s3.object_key = {
            let k = self.sync_s3_key_input.read(cx).value().trim().to_string();
            if k.is_empty() { "velodb/config-sync.json".to_string() } else { k }
        };
        cfg.s3.access_key_id = self.sync_s3_ak_input.read(cx).value().trim().to_string();
        cfg.s3.secret_access_key = self.sync_s3_sk_input.read(cx).value().trim().to_string();
        cfg.s3.session_token = self.sync_s3_token_input.read(cx).value().trim().to_string();

        cfg.webdav.endpoint = self.sync_dav_endpoint_input.read(cx).value().trim().to_string();
        cfg.webdav.username = self.sync_dav_user_input.read(cx).value().trim().to_string();
        cfg.webdav.password = self.sync_dav_pass_input.read(cx).value().to_string();
        cfg.webdav.remote_path = {
            let p = self.sync_dav_path_input.read(cx).value().trim().to_string();
            if p.is_empty() { "velodb/config-sync.json".to_string() } else { p }
        };
        cfg
    }

    pub fn save_sync_settings_from_inputs(&mut self, cx: &mut Context<Self>) {
        self.data.sync_config = self.build_sync_config_from_inputs(cx);
        save_sync_config(&self.data.sync_config);
        self.show_toast("同步配置已保存", cx);
        cx.notify();
    }

    pub fn test_sync_connection_action(&mut self, cx: &mut Context<Self>) {
        if self.data.sync_busy {
            return;
        }
        let cfg = self.build_sync_config_from_inputs(cx);
        self.data.sync_config = cfg.clone();
        save_sync_config(&self.data.sync_config);
        self.data.sync_busy = true;
        self.data.sync_status_msg = Some("正在测试远端连接…".to_string());
        cx.notify();

        let rt = self.rt.clone();
        let this = cx.weak_entity();
        self.window_cx
            .spawn(async move |cx| {
                let res = rt.spawn(async move { test_sync_connection(&cfg).await }).await;
                let _ = this.update_in(cx, |app, _window, cx| {
                    app.data.sync_busy = false;
                    match res {
                        Ok(Ok(info)) => {
                            let msg = format!("✓ {}", info.message);
                            app.data.sync_status_msg = Some(msg.clone());
                            app.show_toast(msg, cx);
                        }
                        Ok(Err(e)) => {
                            let msg = format!("连接测试失败：{e}");
                            app.data.sync_status_msg = Some(msg.clone());
                            app.show_toast(msg, cx);
                        }
                        Err(e) => {
                            let msg = format!("连接测试异常：{e}");
                            app.data.sync_status_msg = Some(msg.clone());
                            app.show_toast(msg, cx);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    pub fn push_sync_now(&mut self, silent: bool, cx: &mut Context<Self>) {
        if self.data.sync_busy {
            return;
        }
        let cfg = self.build_sync_config_from_inputs(cx);
        if !cfg.is_configured() {
            if !silent {
                self.show_toast("请先填写完整的 S3 或 WebDAV 同步配置", cx);
            }
            return;
        }
        self.data.sync_config = cfg.clone();
        save_sync_config(&self.data.sync_config);

        let snippets = if cfg.sync_snippets {
            Some(self.data.snippets.clone())
        } else {
            None
        };
        let favorites = if cfg.sync_favorites {
            Some(&self.data.favorites)
        } else {
            None
        };
        let conn_count = self.data.connections.len();
        let snapshot = SyncSnapshot::build(
            cfg.device_id.clone(),
            self.data.settings.clone(),
            self.data.connections.clone(),
            snippets,
            favorites,
        );

        self.data.sync_busy = true;
        if !silent {
            self.data.sync_status_msg = Some("正在上传全部主机配置与设置…".to_string());
        }
        cx.notify();

        let rt = self.rt.clone();
        let this = cx.weak_entity();
        self.window_cx
            .spawn(async move |cx| {
                let res = rt
                    .spawn(async move { push_snapshot(&cfg, &snapshot).await })
                    .await;
                let _ = this.update_in(cx, |app, _window, cx| {
                    app.data.sync_busy = false;
                    match res {
                        Ok(Ok(())) => {
                            let now = chrono::Utc::now();
                            let local_str = now
                                .with_timezone(&chrono::Local)
                                .format("%Y-%m-%d %H:%M:%S")
                                .to_string();
                            let msg = format!(
                                "✓ 已上传 {conn_count} 个主机连接与全部设置 ({local_str})"
                            );
                            app.data.sync_config.last_sync_at = Some(now);
                            app.data.sync_config.last_sync_status = Some(msg.clone());
                            app.data.sync_status_msg = Some(msg.clone());
                            save_sync_config(&app.data.sync_config);
                            if !silent {
                                app.show_toast(msg, cx);
                            }
                        }
                        Ok(Err(e)) => {
                            let msg = format!("上传配置失败：{e}");
                            app.data.sync_config.last_sync_status = Some(msg.clone());
                            app.data.sync_status_msg = Some(msg.clone());
                            save_sync_config(&app.data.sync_config);
                            if !silent {
                                app.show_toast(msg, cx);
                            }
                        }
                        Err(e) => {
                            let msg = format!("上传任务异常：{e}");
                            app.data.sync_status_msg = Some(msg.clone());
                            if !silent {
                                app.show_toast(msg, cx);
                            }
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    pub fn pull_sync_now(&mut self, silent: bool, cx: &mut Context<Self>) {
        if self.data.sync_busy {
            return;
        }
        let cfg = self.build_sync_config_from_inputs(cx);
        if !cfg.is_configured() {
            if !silent {
                self.show_toast("请先填写完整的 S3 或 WebDAV 同步配置", cx);
            }
            return;
        }
        self.data.sync_config = cfg.clone();
        save_sync_config(&self.data.sync_config);

        self.data.sync_busy = true;
        if !silent {
            self.data.sync_status_msg = Some("正在从远端拉取配置…".to_string());
        }
        cx.notify();

        let rt = self.rt.clone();
        let this = cx.weak_entity();
        self.window_cx
            .spawn(async move |cx| {
                let res = rt.spawn(async move { pull_snapshot(&cfg).await }).await;
                let _ = this.update_in(cx, |app, window, cx| {
                    app.data.sync_busy = false;
                    match res {
                        Ok(Ok(Some(snapshot))) => {
                            let count = app.apply_pulled_snapshot(snapshot, window, cx);
                            let now = chrono::Utc::now();
                            let local_str = now
                                .with_timezone(&chrono::Local)
                                .format("%Y-%m-%d %H:%M:%S")
                                .to_string();
                            let msg = format!(
                                "✓ 已同步 {count} 个主机连接与全部设置 ({local_str})"
                            );
                            app.data.sync_config.last_sync_at = Some(now);
                            app.data.sync_config.last_sync_status = Some(msg.clone());
                            app.data.sync_status_msg = Some(msg.clone());
                            save_sync_config(&app.data.sync_config);
                            if !silent {
                                app.show_toast(msg, cx);
                            }
                        }
                        Ok(Ok(None)) => {
                            let msg = "远端尚无同步配置文件，可先点击「上传到云端」".to_string();
                            app.data.sync_status_msg = Some(msg.clone());
                            if !silent {
                                app.show_toast(msg, cx);
                            }
                        }
                        Ok(Err(e)) => {
                            let msg = format!("拉取配置失败：{e}");
                            app.data.sync_config.last_sync_status = Some(msg.clone());
                            app.data.sync_status_msg = Some(msg.clone());
                            save_sync_config(&app.data.sync_config);
                            if !silent {
                                app.show_toast(msg, cx);
                            }
                        }
                        Err(e) => {
                            let msg = format!("拉取任务异常：{e}");
                            app.data.sync_status_msg = Some(msg.clone());
                            if !silent {
                                app.show_toast(msg, cx);
                            }
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    pub fn apply_pulled_snapshot(
        &mut self,
        snapshot: SyncSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> usize {
        let strategy = self.data.sync_config.pull_strategy;

        // 1. Connections
        let merged_conns = merge_connections(&self.data.connections, &snapshot.connections, strategy);
        let conn_count = merged_conns.len();
        self.data.connections = merged_conns;
        save_connections(&self.data.connections);

        // Refresh active connection display metadata if still present
        if let Some(active) = self.data.active_conn.as_mut() {
            if let Some(updated) = self.data.connections.iter().find(|c| c.id == active.id) {
                active.name = updated.name.clone();
                active.env = updated.env.clone();
                active.label = updated.label.clone();
                active.color = updated.color.clone();
                active.read_only = updated.read_only;
            }
        }

        // 2. Settings
        self.data.settings = snapshot.settings;
        save_settings(&self.data.settings);
        self.sync_component_theme(window, cx);
        let show_lines = self.data.settings.show_line_numbers;
        for sql in self.data.tabs.iter().filter_map(|tab| tab.sql_state.as_ref()) {
            sql.editor
                .update(cx, |ed, cx| ed.set_line_number(show_lines, window, cx));
        }
        for ts in self.data.tabs.iter().filter_map(|tab| tab.table_state.as_ref()) {
            ts.ddl_editor
                .update(cx, |ed, cx| ed.set_line_number(show_lines, window, cx));
        }
        self.inspector_editor_input
            .update(cx, |ed, cx| ed.set_line_number(show_lines, window, cx));

        // 3. Snippets
        if self.data.sync_config.sync_snippets {
            if let Some(remote_snips) = snapshot.snippets {
                self.data.snippets = merge_snippets(&self.data.snippets, &remote_snips, strategy);
                save_snippets(&self.data.snippets);
            }
        }

        // 4. Favorites
        if self.data.sync_config.sync_favorites {
            if let Some(remote_favs) = snapshot.favorites {
                self.data.favorites = merge_favorites(&self.data.favorites, &remote_favs, strategy);
                save_favorites(&self.data.favorites);
                self.sidebar_tree = None;
            }
        }

        conn_count
    }

    pub fn maybe_auto_push_sync(&mut self, cx: &mut Context<Self>) {
        if self.data.sync_config.enabled
            && self.data.sync_config.auto_push_on_change
            && self.data.sync_config.is_configured()
            && !self.data.sync_busy
        {
            self.push_sync_now(true, cx);
        }
    }

    pub fn on_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;

        // Command palette keyboard navigation (works even while search input is focused)
        if self.data.active_modal == ActiveModal::CommandPalette {
            match ks.key.as_str() {
                "down" => {
                    let len = self.build_cmd_palette_items().len();
                    if len > 0 {
                        self.data.cmd_selected_idx = (self.data.cmd_selected_idx + 1) % len;
                        cx.notify();
                    }
                    return;
                }
                "up" => {
                    let len = self.build_cmd_palette_items().len();
                    if len > 0 {
                        self.data.cmd_selected_idx =
                            (self.data.cmd_selected_idx + len - 1) % len;
                        cx.notify();
                    }
                    return;
                }
                "enter" => {
                    self.execute_selected_cmd(window, cx);
                    return;
                }
                "escape" => {
                    self.data.active_modal = ActiveModal::None;
                    self.focus_handle.focus(window, cx);
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }

        if ks.modifiers.platform {
            match ks.key.as_str() {
                "k" => {
                    self.open_command_palette(window, cx);
                    return;
                }
                "f" => {
                    let in_code_editor = self
                        .active_sql_editor()
                        .is_some_and(|ed| ed.focus_handle(cx).is_focused(window))
                        || self
                            .active_ddl_editor()
                            .is_some_and(|ed| ed.focus_handle(cx).is_focused(window))
                        || self.inspector_editor_input.focus_handle(cx).is_focused(window);
                    if self.data.is_connected
                        && self.data.active_modal == ActiveModal::None
                        && !in_code_editor
                    {
                        self.focus_sidebar_search(window, cx);
                        return;
                    }
                }
                "," => {
                    self.data.active_modal = ActiveModal::Settings;
                    cx.notify();
                    return;
                }
                "enter" => {
                    if self.data.is_connected
                        && self.active_tab().map(|t| t.kind == TabKind::Sql).unwrap_or(false)
                    {
                        self.run_active_sql(cx);
                    }
                    return;
                }
                "t" => {
                    if self.data.is_connected {
                        self.add_sql_tab(window, cx);
                    }
                    return;
                }
                "w" => {
                    if self.data.is_connected && !self.data.active_tab_id.is_empty() {
                        let id = self.data.active_tab_id.clone();
                        self.close_tab(&id, window, cx);
                    }
                    return;
                }
                "r" => {
                    if self.data.is_connected {
                        match self.active_tab().map(|t| t.kind) {
                            Some(TabKind::Table) => self.reload_active_table(cx),
                            Some(TabKind::Monitor) => self.refresh_monitor(cx),
                            _ => {}
                        }
                    }
                    return;
                }
                "c" => {
                    if self.data.is_connected
                        && self.data.active_modal == ActiveModal::None
                        && !self.is_any_text_input_focused(window, cx)
                    {
                        self.copy_active_cell(cx);
                        return;
                    }
                }
                _ => {}
            }
        }

        if ks.key == "escape" {
            if self.data.ctx_menu.open
                || self.data.sidebar_ctx_menu.open
                || self.data.tab_ctx_menu.open
                || self.data.db_dropdown_open
                || self.data.schema_dropdown_open
            {
                self.data.ctx_menu.open = false;
                self.data.sidebar_ctx_menu.open = false;
                self.data.tab_ctx_menu.open = false;
                self.data.db_dropdown_open = false;
                self.data.schema_dropdown_open = false;
                cx.notify();
                return;
            }
            if self.data.active_modal != ActiveModal::None {
                self.data.active_modal = ActiveModal::None;
                cx.notify();
                return;
            }
            if self.sidebar_filter_input.focus_handle(cx).is_focused(window) {
                if !self.data.sidebar_filter.is_empty() {
                    self.data.sidebar_filter.clear();
                    self.sidebar_filter_input
                        .update(cx, |s, cx| s.set_value("", window, cx));
                }
                self.focus_handle.focus(window, cx);
                cx.notify();
                return;
            }
            if self.data.drawer_open && !self.is_any_text_input_focused(window, cx) {
                self.data.drawer_open = false;
                cx.notify();
                return;
            }
        }

        // Data / SQL result grid keyboard navigation when no input or modal is focused
        if self.data.is_connected
            && self.data.active_modal == ActiveModal::None
            && !ks.modifiers.platform
            && !ks.modifiers.control
            && !ks.modifiers.alt
            && !self.is_any_text_input_focused(window, cx)
        {
            match ks.key.as_str() {
                "up" => self.move_grid_selection(-1, 0, window, cx),
                "down" => self.move_grid_selection(1, 0, window, cx),
                "left" => self.move_grid_selection(0, -1, window, cx),
                "right" => self.move_grid_selection(0, 1, window, cx),
                "enter" => {
                    if matches!(
                        self.active_tab().map(|t| t.kind),
                        Some(TabKind::Table | TabKind::Sql)
                    ) {
                        self.data.drawer_open = true;
                        self.data.drawer_tab = DrawerTab::Cell;
                        self.sync_inspector_for_active_cell(window, cx);
                        cx.notify();
                    }
                }
                _ => {}
            }
        }
    }

    pub fn render_titlebar(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let conn = self.data.active_conn.as_ref();
        let conn_color = conn
            .and_then(|c| crate::ui::theme::parse_hex(&c.color))
            .unwrap_or(t.accent);

        TitleBar::new()
            .h(px(44.0))
            .bg(t.titlebar_bg)
            .border_b_1()
            .border_color(t.border)
            .pl(px(78.0)) // Space for macOS native traffic lights
            .pr(px(12.0))
            .child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .when(self.data.is_connected, |d| {
                                let sidebar_icon_color = if self.data.sidebar_open {
                                    t.accent
                                } else {
                                    t.text_2
                                };
                                let monitor_tip_theme = *t;
                                let new_query_tip_theme = *t;
                                d.child(
                                    div()
                                        .id("toggle-sidebar-btn")
                                        .h(px(28.0))
                                        .w(px(28.0))
                                        .rounded(px(6.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .bg(if self.data.sidebar_open {
                                            t.accent_soft
                                        } else {
                                            gpui::transparent_black()
                                        })
                                        .hover(|s| s.bg(t.surface_2))
                                        .on_mouse_down(MouseButton::Left, |_, window, _| {
                                            window.prevent_default();
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.data.sidebar_open = !this.data.sidebar_open;
                                            cx.notify();
                                        }))
                                        .child(hgi("sidebar-left", 15.0, sidebar_icon_color)),
                                )
                                .child(
                                    div()
                                        .id("conn-pill-btn")
                                        .h(px(28.0))
                                        .px(px(9.0))
                                        .rounded(px(6.0))
                                        .bg(t.surface_1)
                                        .border_1()
                                        .border_color(t.border)
                                        .flex()
                                        .items_center()
                                        .gap(px(7.0))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(t.surface_2).border_color(t.border_strong))
                                        .on_mouse_down(MouseButton::Left, |_, window, _| {
                                            window.prevent_default();
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.data.active_modal = ActiveModal::ConnectionManager;
                                            cx.notify();
                                        }))
                                        .child(
                                            div().w(px(7.0)).h(px(7.0)).rounded_full().bg(conn_color),
                                        )
                                        .child(
                                            div()
                                                .text_size(t.fs_md())
                                                .font_weight(gpui::FontWeight::MEDIUM)
                                                .text_color(t.text_1)
                                                .child(
                                                    conn.map(|c| c.name.clone())
                                                        .unwrap_or_else(|| "velodb".into()),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .px(px(5.0))
                                                .py(px(1.0))
                                                .rounded(px(4.0))
                                                .bg(with_alpha(conn_color, 0.14))
                                                .text_color(conn_color)
                                                .text_size(t.fs_xs())
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child(
                                                    conn.map(|c| c.label.clone())
                                                        .unwrap_or_else(|| "DB".into()),
                                                ),
                                        )
                                        .child(hgi("arrow-down", 12.0, t.text_3)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(4.0))
                                        .flex_shrink_0()
                                        .child(
                                            div()
                                                .id("open-monitor-btn")
                                                .h(px(28.0))
                                                .w(px(28.0))
                                                .rounded(px(6.0))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .cursor_pointer()
                                                .hover(|s| s.bg(t.surface_2))
                                                .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                                                .tooltip(move |window, cx| {
                                                    build_sidebar_tooltip(
                                                        "实时监控".to_string(),
                                                        monitor_tip_theme,
                                                        window,
                                                        cx,
                                                    )
                                                })
                                                .on_mouse_down(MouseButton::Left, |_, window, _| {
                                                    window.prevent_default();
                                                })
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.open_monitor_tab(cx);
                                                }))
                                                .child(hgi("activity", 15.0, t.text_2)),
                                        )
                                        .child(
                                            div()
                                                .id("new-sql-tab-btn")
                                                .h(px(28.0))
                                                .w(px(28.0))
                                                .rounded(px(6.0))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .cursor_pointer()
                                                .hover(|s| s.bg(t.surface_2))
                                                .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                                                .tooltip(move |window, cx| {
                                                    build_sidebar_tooltip(
                                                        "新建 SQL 查询 (⌘T)".to_string(),
                                                        new_query_tip_theme,
                                                        window,
                                                        cx,
                                                    )
                                                })
                                                .on_mouse_down(MouseButton::Left, |_, window, _| {
                                                    window.prevent_default();
                                                })
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    cx.stop_propagation();
                                                    this.add_sql_tab(window, cx);
                                                }))
                                                .child(hgi("file-code", 15.0, t.text_2)),
                                        ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .id("cmd-trigger-btn")
                            .w(px(440.0))
                            .h(t.control_h())
                            .px(px(12.0))
                            .rounded(px(6.0))
                            .bg(t.surface_1)
                            .border_1()
                            .border_color(t.border)
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.surface_2).border_color(t.border_strong))
                            .on_mouse_down(MouseButton::Left, |_, window, _| {
                                window.prevent_default();
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.open_command_palette(window, cx);
                            }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .text_size(t.fs_md())
                                    .text_color(t.text_3)
                                    .child(hgi("search", 13.0, t.text_3))
                                    .child("搜索表、命令或切换连接..."),
                            )
                            .child(
                                div()
                                    .px(px(5.0))
                                    .py(px(1.0))
                                    .rounded(px(4.0))
                                    .bg(t.surface_2)
                                    .border_1()
                                    .border_color(t.border)
                                    .text_size(t.fs_xs())
                                    .text_color(t.text_2)
                                    .child("⌘K"),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(4.0))
                            .when(self.data.is_connected, |d| {
                                let inspector_icon_color = if self.data.drawer_open {
                                    t.accent
                                } else {
                                    t.text_2
                                };
                                d.child(
                                    div()
                                        .id("toggle-inspector-btn")
                                        .h(px(28.0))
                                        .w(px(28.0))
                                        .rounded(px(6.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .bg(if self.data.drawer_open {
                                            t.accent_soft
                                        } else {
                                            gpui::transparent_black()
                                        })
                                        .hover(|s| s.bg(t.surface_2))
                                        .on_mouse_down(MouseButton::Left, |_, window, _| {
                                            window.prevent_default();
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.data.drawer_open = !this.data.drawer_open;
                                            if this.data.drawer_open {
                                                this.sync_inspector_for_active_cell(window, cx);
                                            }
                                            cx.notify();
                                        }))
                                        .child(hgi("sidebar-right", 15.0, inspector_icon_color)),
                                )
                                .child(div().w(px(1.0)).h(px(16.0)).bg(t.border_strong).mx(px(4.0)))
                            })
                            .child(
                                div()
                                    .id("open-settings-btn")
                                    .h(px(28.0))
                                    .w(px(28.0))
                                    .rounded(px(6.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.surface_2))
                                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                                        window.prevent_default();
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.data.active_modal = ActiveModal::Settings;
                                        cx.notify();
                                    }))
                                    .child(hgi("settings", 15.0, t.text_2)),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub fn render_statusbar(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        if !self.data.is_connected {
            return div().into_any_element();
        }
        let ssh_label = self
            .data
            .active_conn
            .as_ref()
            .and_then(|c| c.ssh.as_ref())
            .filter(|s| s.enabled)
            .map(|_| "SSH")
            .unwrap_or("Direct");

        let is_mysql = self
            .data
            .active_conn
            .as_ref()
            .map(|c| c.driver == crate::adapter::DbDriver::Mysql)
            .unwrap_or(false);
        let table_info = self
            .active_tab()
            .and_then(|tab| tab.table_state.as_ref())
            .map(|ts| format!("{}.{}", ts.schema, ts.table))
            .unwrap_or_else(|| {
                if self.data.current_db.is_empty() {
                    "未选择数据库".to_string()
                } else if is_mysql {
                    format!("db: {}", self.data.current_db)
                } else {
                    format!(
                        "db: {} · schema: {}",
                        self.data.current_db, self.data.current_schema
                    )
                }
            });

        div()
            .h(px(26.0))
            .flex_shrink_0()
            .bg(t.titlebar_bg)
            .border_t_1()
            .border_color(t.border)
            .px(px(10.0))
            .flex()
            .items_center()
            .justify_between()
            .text_size(t.fs_sm())
            .text_color(t.text_3)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .text_color(t.text_2)
                            .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(t.green))
                            .child(self.data.server_version.clone()),
                    )
                    .child(div().w(px(1.0)).h(px(11.0)).bg(t.border_strong))
                    .child(ssh_label)
                    .child(div().w(px(1.0)).h(px(11.0)).bg(t.border_strong))
                    .child(table_info),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("safe-mode-btn")
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .cursor_pointer()
                            .text_color(if self.data.settings.safe_mode {
                                t.green
                            } else {
                                t.amber
                            })
                            .hover(|s| s.bg(t.surface_2))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.data.settings.safe_mode = !this.data.settings.safe_mode;
                                save_settings(&this.data.settings);
                                let msg = if this.data.settings.safe_mode {
                                    "Safe Mode 已开启"
                                } else {
                                    "已切换为直接执行模式"
                                };
                                this.show_toast(msg, cx);
                            }))
                            .child(hgi(
                                if self.data.settings.safe_mode {
                                    "shield-check"
                                } else {
                                    "alert"
                                },
                                12.0,
                                if self.data.settings.safe_mode {
                                    t.green
                                } else {
                                    t.amber
                                },
                            ))
                            .child(if self.data.settings.safe_mode {
                                "安全模式"
                            } else {
                                "直连模式"
                            }),
                    )
                    .child(div().w(px(1.0)).h(px(11.0)).bg(t.border_strong))
                    .child(
                        div()
                            .id("mcp-status-btn")
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .cursor_pointer()
                            .text_color(if self.data.settings.mcp_enabled {
                                t.cyan
                            } else {
                                t.text_3
                            })
                            .hover(|s| s.bg(t.surface_2))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.data.active_modal = ActiveModal::McpConfig;
                                cx.notify();
                            }))
                            .child(hgi(
                                "plug",
                                12.0,
                                if self.data.settings.mcp_enabled {
                                    t.cyan
                                } else {
                                    t.text_3
                                },
                            ))
                            .child(if self.data.settings.mcp_enabled {
                                if self.data.settings.mcp_read_only {
                                    "MCP: 只读"
                                } else {
                                    "MCP: 读写"
                                }
                            } else {
                                "MCP: 已关闭"
                            }),
                    )
                    .child(div().w(px(1.0)).h(px(11.0)).bg(t.border_strong))
                    .child("UTF-8"),
            )
            .into_any_element()
    }
}

impl Render for VeloDbApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme(window);

        let main_content = if !self.data.is_connected {
            self.render_launcher(&t, cx)
        } else {
            let active_kind = self.active_tab().map(|tab| tab.kind);

            let tab_body = match active_kind {
                Some(TabKind::Table) => self.render_table_view(&t, cx),
                Some(TabKind::Sql) => self.render_sql_workbench(&t, cx),
                Some(TabKind::Monitor) => self.render_monitor_view(&t, cx),
                None => div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(8.0))
                    .text_color(t.text_3)
                    .child(div().opacity(0.45).child(hgi("database", 28.0, t.text_3)))
                    .child(
                        div()
                            .text_size(t.fs_md())
                            .child(if self.data.current_db.is_empty() {
                                "请先在左侧选择数据库，或点击左上角查询按钮 (⌘T) 新建 SQL 查询"
                            } else {
                                "从左侧选择数据表，或点击左上角查询按钮 (⌘T) 新建 SQL 查询"
                            }),
                    )
                    .into_any_element(),
            };

            div()
                .flex_1()
                .flex()
                .min_h_0()
                .relative()
                .when(self.data.sidebar_open, |d| {
                    d.child(self.render_sidebar(&t, cx))
                })
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .min_h_0()
                        .relative()
                        .child(self.render_tabbar(&t, cx))
                        .child(tab_body)
                        .child(self.render_staged_bar(&t, cx)),
                )
                .when(
                    self.data.drawer_open
                        && matches!(active_kind, Some(TabKind::Table | TabKind::Sql)),
                    |d| d.child(self.render_inspector(&t, cx)),
                )
                .into_any_element()
        };

        div()
            .id("velodb-root")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    let mut changed = false;
                    if this.data.ctx_menu.open {
                        this.data.ctx_menu.open = false;
                        changed = true;
                    }
                    if this.data.sidebar_ctx_menu.open {
                        this.data.sidebar_ctx_menu.open = false;
                        changed = true;
                    }
                    if this.data.tab_ctx_menu.open {
                        this.data.tab_ctx_menu.open = false;
                        changed = true;
                    }
                    if this.data.db_dropdown_open {
                        this.data.db_dropdown_open = false;
                        changed = true;
                    }
                    if this.data.schema_dropdown_open {
                        this.data.schema_dropdown_open = false;
                        changed = true;
                    }
                    if this.data.open_dropdown.is_some() {
                        this.data.open_dropdown = None;
                        changed = true;
                    }
                    if changed {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                if this.data.sidebar_resizing {
                    let new_w = f32::from(ev.position.x).clamp(180.0, 520.0);
                    if (new_w - this.data.settings.sidebar_width).abs() > 0.5 {
                        this.data.settings.sidebar_width = new_w;
                        cx.notify();
                    }
                } else if this.data.inspector_resizing {
                    let win_w = f32::from(window.viewport_size().width);
                    let new_w = (win_w - f32::from(ev.position.x)).clamp(260.0, 900.0);
                    if (new_w - this.data.settings.inspector_width).abs() > 0.5 {
                        this.data.settings.inspector_width = new_w;
                        cx.notify();
                    }
                } else if let Some((start_y, start_h)) = this.data.sql_split_resizing {
                    let new_h = (start_h + (f32::from(ev.position.y) - start_y)).clamp(120.0, 560.0);
                    if (new_h - this.data.settings.sql_editor_height).abs() > 0.5 {
                        this.data.settings.sql_editor_height = new_h;
                        cx.notify();
                    }
                } else if let Some(rs) = this.data.col_resizing.clone() {
                    let delta = f32::from(ev.position.x) - rs.start_x;
                    let new_w = (rs.start_width + delta).clamp(64.0, 800.0);
                    match rs.target {
                        crate::ui::state::ColResizeTarget::Table { col_name } => {
                            if let Some(ts) =
                                this.active_tab_mut().and_then(|t| t.table_state.as_mut())
                            {
                                let cur = ts
                                    .col_widths
                                    .get(&col_name)
                                    .copied()
                                    .unwrap_or(rs.start_width);
                                if (new_w - cur).abs() > 0.5 {
                                    ts.col_widths.insert(col_name, new_w);
                                    cx.notify();
                                }
                            }
                        }
                        crate::ui::state::ColResizeTarget::Sql { col_idx } => {
                            if let Some(ss) =
                                this.active_tab_mut().and_then(|t| t.sql_state.as_mut())
                            {
                                let cur = ss
                                    .col_widths
                                    .get(&col_idx)
                                    .copied()
                                    .unwrap_or(rs.start_width);
                                if (new_w - cur).abs() > 0.5 {
                                    ss.col_widths.insert(col_idx, new_w);
                                    cx.notify();
                                }
                            }
                        }
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    let mut changed = false;
                    let mut persist = false;
                    if this.data.sidebar_resizing {
                        this.data.sidebar_resizing = false;
                        persist = true;
                        changed = true;
                    }
                    if this.data.inspector_resizing {
                        this.data.inspector_resizing = false;
                        persist = true;
                        changed = true;
                    }
                    if this.data.sql_split_resizing.is_some() {
                        this.data.sql_split_resizing = None;
                        persist = true;
                        changed = true;
                    }
                    if this.data.col_resizing.is_some() {
                        this.data.col_resizing = None;
                        changed = true;
                    }
                    if persist {
                        save_settings(&this.data.settings);
                    }
                    if changed {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    let mut changed = false;
                    let mut persist = false;
                    if this.data.sidebar_resizing {
                        this.data.sidebar_resizing = false;
                        persist = true;
                        changed = true;
                    }
                    if this.data.inspector_resizing {
                        this.data.inspector_resizing = false;
                        persist = true;
                        changed = true;
                    }
                    if this.data.sql_split_resizing.is_some() {
                        this.data.sql_split_resizing = None;
                        persist = true;
                        changed = true;
                    }
                    if this.data.col_resizing.is_some() {
                        this.data.col_resizing = None;
                        changed = true;
                    }
                    if persist {
                        save_settings(&this.data.settings);
                    }
                    if changed {
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(t.window_bg)
            .text_color(t.text_1)
            .text_size(t.fs_md())
            .child(self.render_titlebar(&t, cx))
            .child(main_content)
            .child(self.render_statusbar(&t, cx))
            .when(self.data.ctx_menu.open, |d| {
                d.child(self.render_context_menu(&t, window, cx))
            })
            .when(self.data.sidebar_ctx_menu.open, |d| {
                d.child(self.render_sidebar_context_menu(&t, window, cx))
            })
            .when(self.data.tab_ctx_menu.open, |d| {
                d.child(self.render_tab_context_menu(&t, window, cx))
            })
            .when(self.data.active_modal != ActiveModal::None, |d| {
                d.child(self.render_active_modal(&t, cx))
            })
            .when_some(self.pending_connection.as_ref(), |d, load| {
                d.child(
                    div()
                        .absolute()
                        .bottom(px(38.0))
                        .left(px(16.0))
                        .w(px(258.0))
                        .px(px(12.0))
                        .py(px(9.0))
                        .rounded(px(7.0))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border_strong)
                        .shadow_lg()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(div().truncate().text_size(t.fs_md()).child(load.bar.read(cx).label.clone()))
                        .child(load.bar.clone()),
                )
            })
            .when_some(self.data.toast.clone(), |d, msg| {
                d.child(
                    div()
                        .absolute()
                        .bottom(px(44.0))
                        .right(px(24.0))
                        .px(px(14.0))
                        .py(px(9.0))
                        .rounded(px(9.0))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border_strong)
                        .shadow_lg()
                        .text_size(t.fs_md())
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(t.text_1)
                        .child(msg),
                )
            })
    }
}

pub(crate) fn quote_ident_for_driver(driver: DbDriver, ident: &str) -> String {
    match driver {
        DbDriver::Mysql | DbDriver::Mariadb => crate::adapter::mysql::quote_ident(ident),
        _ => crate::adapter::postgres::quote_ident(ident),
    }
}

pub(crate) fn build_qualified_table_sql(driver: DbDriver, schema: &str, name: &str) -> String {
    let q_name = quote_ident_for_driver(driver, name);
    if schema.is_empty() {
        q_name
    } else {
        format!("{}.{q_name}", quote_ident_for_driver(driver, schema))
    }
}

pub(crate) fn build_rename_table_sql(
    driver: DbDriver,
    schema: &str,
    old_name: &str,
    new_name: &str,
) -> String {
    let old_q = build_qualified_table_sql(driver, schema, old_name);
    match driver {
        DbDriver::Mysql | DbDriver::Mariadb => {
            let new_q = build_qualified_table_sql(driver, schema, new_name);
            format!("RENAME TABLE {old_q} TO {new_q};")
        }
        _ => {
            let new_ident = quote_ident_for_driver(driver, new_name);
            format!("ALTER TABLE {old_q} RENAME TO {new_ident};")
        }
    }
}

pub(crate) fn build_danger_table_sql(
    driver: DbDriver,
    action: TableDangerAction,
    schema: &str,
    table: &str,
) -> String {
    let qualified = build_qualified_table_sql(driver, schema, table);
    match action {
        TableDangerAction::Truncate => format!("TRUNCATE TABLE {qualified};"),
        TableDangerAction::Drop => format!("DROP TABLE {qualified};"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_ddl_sql_builders_quote_per_driver() {
        assert_eq!(
            build_qualified_table_sql(DbDriver::Postgres, "public", "users"),
            "\"public\".\"users\""
        );
        assert_eq!(
            build_qualified_table_sql(DbDriver::Mysql, "app_db", "users"),
            "`app_db`.`users`"
        );
        assert_eq!(
            build_rename_table_sql(DbDriver::Postgres, "public", "old_tbl", "new_tbl"),
            "ALTER TABLE \"public\".\"old_tbl\" RENAME TO \"new_tbl\";"
        );
        assert_eq!(
            build_rename_table_sql(DbDriver::Mysql, "app_db", "old_tbl", "new_tbl"),
            "RENAME TABLE `app_db`.`old_tbl` TO `app_db`.`new_tbl`;"
        );
        assert_eq!(
            build_danger_table_sql(
                DbDriver::Postgres,
                TableDangerAction::Truncate,
                "public",
                "logs"
            ),
            "TRUNCATE TABLE \"public\".\"logs\";"
        );
        assert_eq!(
            build_danger_table_sql(DbDriver::Mysql, TableDangerAction::Drop, "app_db", "logs"),
            "DROP TABLE `app_db`.`logs`;"
        );
    }
}
