use crate::adapter::{
    mysql::generate_mysql_staged_sql,
    postgres::{export_rows, generate_staged_sql},
    DbDriver, ExportFormat, SchemaObjectType, SslMode,
};
use crate::config::sync::{save_sync_config, SyncProvider, SyncPullStrategy};
use crate::config::{save_connections, save_settings, save_snippets, AppSettings, SqlSnippet};
use crate::ui::app::{build_danger_table_sql, build_rename_table_sql, VeloDbApp};
use crate::ui::icons::{db_driver_icon, hgi, macos_icon};
use crate::ui::sidebar::{build_sidebar_tooltip, matches_search_query, SIDEBAR_TOOLTIP_DELAY};
use crate::ui::state::{ActiveModal, SettingsCategory, TableDangerAction};
use crate::ui::theme::{parse_hex, with_alpha, VeloTheme};
use gpui::{
    deferred, div, prelude::*, px, AnyElement, Context, InteractiveElement, MouseButton,
    ParentElement, Styled,
};
use gpui_component::input::Input;
use gpui_component::scroll::ScrollableElement;

impl VeloDbApp {
    pub fn render_active_modal(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let content: AnyElement = match &self.data.active_modal {
            ActiveModal::None => return div().into_any_element(),
            ActiveModal::CommandPalette => self.modal_cmd_palette(t, cx),
            ActiveModal::ConnectionManager => self.modal_conn_manager(t, cx),
            ActiveModal::NewConnection { step } => self.modal_new_conn(*step, t, cx),
            ActiveModal::McpConfig => self.modal_mcp(t, cx),
            ActiveModal::ColumnsPicker => self.modal_columns_picker(t, cx),
            ActiveModal::PreviewSql => self.modal_preview_sql(t, cx),
            ActiveModal::ExportImport { is_import } => self.modal_export_import(*is_import, t, cx),
            ActiveModal::SnippetsManager => self.modal_snippets(t, cx),
            ActiveModal::Settings => self.modal_settings(t, cx),
            ActiveModal::RenameTable { schema, table } => {
                self.modal_rename_table(schema, table, t, cx)
            }
            ActiveModal::ConfirmTableAction {
                action,
                schema,
                table,
            } => self.modal_confirm_table_action(*action, schema, table, t, cx),
        };

        div()
            .id("modal-backdrop")
            .absolute()
            .inset_0()
            .bg(gpui::hsla(0.0, 0.0, 0.0, 0.48))
            .flex()
            .items_start()
            .justify_center()
            .pt(px(56.0))
            .on_click(cx.listener(|this, _, _, cx| {
                this.data.active_modal = ActiveModal::None;
                cx.notify();
            }))
            .child(content)
            .into_any_element()
    }

    fn modal_shell(
        &self,
        id: &'static str,
        width: f32,
        title: String,
        body: AnyElement,
        footer: Option<AnyElement>,
        t: &VeloTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .w(px(width))
            .max_h(px(720.0))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border_strong)
            .rounded(px(12.0))
            .shadow_lg()
            .overflow_hidden()
            .flex()
            .flex_col()
            .on_click(cx.listener(|_, _, _, cx| {
                cx.stop_propagation();
            }))
            .child(
                div()
                    .px(px(16.0))
                    .py(px(13.0))
                    .border_b_1()
                    .border_color(t.border)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(t.fs_lg())
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .id("modal-close-x")
                            .w(px(24.0))
                            .h(px(24.0))
                            .rounded(px(6.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .text_color(t.text_2)
                            .hover(|s| s.bg(t.surface_2))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.data.active_modal = ActiveModal::None;
                                cx.notify();
                            }))
                            .child(hgi("cancel", 14.0, t.text_2)),
                    ),
            )
            .child(div().flex_1().overflow_y_scrollbar().child(body))
            .when_some(footer, |d, f| {
                d.child(
                    div()
                        .px(px(16.0))
                        .py(px(12.0))
                        .border_t_1()
                        .border_color(t.border)
                        .bg(t.surface_1)
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .child(f),
                )
            })
            .into_any_element()
    }

    pub fn build_cmd_palette_items(
        &self,
    ) -> Vec<(
        String,
        &'static str,
        Box<dyn Fn(&mut Self, &mut gpui::Window, &mut Context<Self>)>,
    )> {
        let q = self.data.cmd_query.trim().to_string();
        let mut items: Vec<(
            String,
            &'static str,
            Box<dyn Fn(&mut Self, &mut gpui::Window, &mut Context<Self>)>,
        )> = Vec::new();

        // Collect openable objects (tables, views, materialized views), pinning favorites first.
        let mut fav_objects = Vec::new();
        let mut other_objects = Vec::new();
        for objects in self.data.schema_objects.values() {
            for obj in objects {
                let kind_label = match obj.object_type {
                    SchemaObjectType::Table => "数据表",
                    SchemaObjectType::View => "视图",
                    SchemaObjectType::MaterializedView => "物化视图",
                    _ => continue,
                };
                if !matches_search_query(&obj.schema, &obj.name, &q)
                    && !(q.is_empty() || kind_label.contains(&q))
                {
                    continue;
                }
                let is_fav = self.is_favorite(&obj.schema, &obj.name);
                let sch = obj.schema.clone();
                let name = obj.name.clone();
                let label = if is_fav {
                    format!("★ {kind_label}：{sch}.{name}")
                } else {
                    format!("{kind_label}：{sch}.{name}")
                };
                let badge = if is_fav { "收藏" } else { "跳转" };
                let entry: (
                    String,
                    &'static str,
                    Box<dyn Fn(&mut Self, &mut gpui::Window, &mut Context<Self>)>,
                ) = (
                    label,
                    badge,
                    Box::new(move |app, window, cx| {
                        app.open_table_tab(&sch, &name, None, window, cx)
                    }),
                );
                if is_fav {
                    fav_objects.push(entry);
                } else {
                    other_objects.push(entry);
                }
            }
        }
        items.extend(fav_objects);
        items.extend(other_objects);

        let mut cmd_items: Vec<(
            String,
            &'static str,
            Box<dyn Fn(&mut Self, &mut gpui::Window, &mut Context<Self>)>,
        )> = Vec::new();
        cmd_items.push((
            "打开实时监控 (pg_stat_activity)".into(),
            "监控",
            Box::new(|app, _, cx| app.open_monitor_tab(cx)),
        ));
        cmd_items.push((
            "新建 SQL 查询标签页".into(),
            "查询",
            Box::new(|app, window, cx| app.add_sql_tab(window, cx)),
        ));
        cmd_items.push((
            "查看本地 MCP 服务配置".into(),
            "MCP",
            Box::new(|app, _, cx| {
                app.data.active_modal = ActiveModal::McpConfig;
                cx.notify();
            }),
        ));
        cmd_items.push((
            "打开偏好设置 (⌘,)".into(),
            "设置",
            Box::new(|app, _, cx| {
                app.data.active_modal = ActiveModal::Settings;
                cx.notify();
            }),
        ));
        cmd_items.push((
            "云端配置同步 (S3 / WebDAV)".into(),
            "同步",
            Box::new(|app, _, cx| {
                app.data.settings_cat = SettingsCategory::Sync;
                app.data.active_modal = ActiveModal::Settings;
                cx.notify();
            }),
        ));
        cmd_items.push((
            "新建数据库连接...".into(),
            "连接",
            Box::new(|app, window, cx| {
                app.open_new_connection_modal(window, cx);
            }),
        ));
        cmd_items.push((
            "切换浅色 / 深色 / 跟随系统主题".into(),
            "外观",
            Box::new(|app, window, cx| {
                app.data.settings.theme = match app.data.settings.theme.as_str() {
                    "system" => "light",
                    "light" => "dark",
                    _ => "system",
                }
                .to_string();
                app.sync_component_theme(window, cx);
                save_settings(&app.data.settings);
                app.show_toast(format!("主题已切换：{}", app.data.settings.theme), cx);
            }),
        ));
        if self.data.is_connected {
            cmd_items.push((
                "断开连接并返回首页".into(),
                "视图",
                Box::new(|app, _, cx| app.disconnect(cx)),
            ));
        }

        let q_lower = q.to_lowercase();
        items.extend(
            cmd_items
                .into_iter()
                .filter(|(label, _, _)| q_lower.is_empty() || label.to_lowercase().contains(&q_lower)),
        );
        items
    }

    fn modal_rename_table(
        &self,
        schema: &str,
        table: &str,
        t: &VeloTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let new_preview = self.rename_table_input.read(cx).value().trim().to_string();
        let preview_target = if new_preview.is_empty() {
            table
        } else {
            new_preview.as_str()
        };
        let sql_preview =
            build_rename_table_sql(self.active_driver(), schema, table, preview_target);

        let body = div()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .text_size(t.fs_md())
                    .text_color(t.text_2)
                    .child(format!("为表 {schema}.{table} 输入新名称：")),
            )
            .child(
                div()
                    .rounded(px(7.0))
                    .bg(t.surface_1)
                    .border_1()
                    .border_color(t.border_strong)
                    .px(px(10.0))
                    .py(px(2.0))
                    .child(
                        Input::new(&self.rename_table_input)
                            .appearance(false)
                            .px_0()
                            .py_0()
                            .h(t.control_h())
                            .text_size(t.fs_md()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(t.fs_sm())
                            .text_color(t.text_3)
                            .child("将执行的 SQL："),
                    )
                    .child(
                        div()
                            .px(px(10.0))
                            .py(px(8.0))
                            .rounded(px(6.0))
                            .bg(t.surface_2)
                            .border_1()
                            .border_color(t.border)
                            .font_family(self.data.settings.font_family.clone())
                            .text_size(t.fs_sm())
                            .text_color(t.text_1)
                            .child(sql_preview),
                    ),
            )
            .into_any_element();

        let footer = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .id("rename-table-cancel")
                    .px(px(12.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(t.surface_2)
                    .text_size(t.fs_md())
                    .text_color(t.text_1)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.surface_3))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.data.active_modal = ActiveModal::None;
                        cx.notify();
                    }))
                    .child("取消"),
            )
            .child(
                div()
                    .id("rename-table-confirm")
                    .px(px(14.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(t.accent)
                    .text_size(t.fs_md())
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(gpui::white())
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.9))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.confirm_rename_table(window, cx);
                    }))
                    .child("确认重命名"),
            )
            .into_any_element();

        self.modal_shell(
            "rename-table-modal",
            460.0,
            format!("重命名表 · {table}"),
            body,
            Some(footer),
            t,
            cx,
        )
    }

    fn modal_confirm_table_action(
        &self,
        action: TableDangerAction,
        schema: &str,
        table: &str,
        t: &VeloTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (title, desc, confirm_label) = match action {
            TableDangerAction::Truncate => (
                format!("清空表 · {table}"),
                format!("此操作将永久清空表 {schema}.{table} 中的所有数据行且不可撤销。"),
                "确认清空 (TRUNCATE)",
            ),
            TableDangerAction::Drop => (
                format!("删除表 · {table}"),
                format!("此操作将永久删除表 {schema}.{table} 及其全部数据与索引，且不可撤销。"),
                "确认删除 (DROP TABLE)",
            ),
        };
        let sql_preview = build_danger_table_sql(self.active_driver(), action, schema, table);
        let sch_exec = schema.to_string();
        let tbl_exec = table.to_string();

        let body = div()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .p(px(10.0))
                    .rounded(px(7.0))
                    .bg(t.red_soft)
                    .border_1()
                    .border_color(with_alpha(t.red, 0.35))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(hgi("alert", 16.0, t.red))
                    .child(
                        div()
                            .flex_1()
                            .text_size(t.fs_md())
                            .text_color(t.text_1)
                            .child(desc),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(t.fs_sm())
                            .text_color(t.text_3)
                            .child("将执行的 SQL："),
                    )
                    .child(
                        div()
                            .px(px(10.0))
                            .py(px(8.0))
                            .rounded(px(6.0))
                            .bg(t.surface_2)
                            .border_1()
                            .border_color(t.border)
                            .font_family(self.data.settings.font_family.clone())
                            .text_size(t.fs_sm())
                            .text_color(t.red)
                            .child(sql_preview),
                    ),
            )
            .into_any_element();

        let footer = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .id("danger-table-cancel")
                    .px(px(12.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(t.surface_2)
                    .text_size(t.fs_md())
                    .text_color(t.text_1)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.surface_3))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.data.active_modal = ActiveModal::None;
                        cx.notify();
                    }))
                    .child("取消"),
            )
            .child(
                div()
                    .id("danger-table-confirm")
                    .px(px(14.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(t.red)
                    .text_size(t.fs_md())
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(gpui::white())
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.9))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.execute_table_danger_action(
                            action,
                            sch_exec.clone(),
                            tbl_exec.clone(),
                            cx,
                        );
                    }))
                    .child(confirm_label),
            )
            .into_any_element();

        self.modal_shell(
            "confirm-table-action-modal",
            480.0,
            title,
            body,
            Some(footer),
            t,
            cx,
        )
    }

    pub fn execute_selected_cmd(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let items = self.build_cmd_palette_items();
        if items.is_empty() {
            return;
        }
        let idx = self.data.cmd_selected_idx.min(items.len() - 1);
        let (_, _, act) = items.into_iter().nth(idx).unwrap();
        self.data.active_modal = ActiveModal::None;
        act(self, window, cx);
    }

    fn modal_cmd_palette(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let filtered = self.build_cmd_palette_items();
        let selected_idx = if filtered.is_empty() {
            0
        } else {
            self.data.cmd_selected_idx.min(filtered.len() - 1)
        };

        div()
            .id("cmd-modal-card")
            .w(px(580.0))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border_strong)
            .rounded(px(12.0))
            .shadow_lg()
            .overflow_hidden()
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                div()
                    .px(px(14.0))
                    .py(px(10.0))
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        Input::new(&self.cmd_search_input)
                            .appearance(false)
                            .px_0()
                            .py_0()
                            .h(t.control_h())
                            .text_size(t.fs_md()),
                    ),
            )
            .child(
                div()
                    .max_h(px(340.0))
                    .overflow_y_scrollbar()
                    .p(px(6.0))
                    .flex()
                    .flex_col()
                    .when(filtered.is_empty(), |d| {
                        d.child(
                            div()
                                .p(px(18.0))
                                .text_center()
                                .text_size(t.fs_md())
                                .text_color(t.text_3)
                                .child("没有匹配的命令或数据表"),
                        )
                    })
                    .children(filtered.into_iter().enumerate().map(|(i, (label, tag, act))| {
                        let is_sel = i == selected_idx;
                        div()
                            .id(("cmd-item", i))
                            .px(px(12.0))
                            .py(px(8.0))
                            .rounded(px(6.0))
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_size(t.fs_md())
                            .bg(if is_sel {
                                t.accent_soft
                            } else {
                                gpui::transparent_black()
                            })
                            .text_color(if is_sel { t.text_1 } else { t.text_2 })
                            .cursor_pointer()
                            .hover(|s| s.bg(t.accent_soft).text_color(t.text_1))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.data.active_modal = ActiveModal::None;
                                act(this, window, cx);
                            }))
                            .child(label)
                            .child(
                                div()
                                    .px(px(5.0))
                                    .py(px(1.0))
                                    .rounded(px(4.0))
                                    .bg(t.surface_2)
                                    .border_1()
                                    .border_color(t.border)
                                    .text_size(t.fs_xs())
                                    .child(tag),
                            )
                    })),
            )
            .child(
                div()
                    .px(px(12.0))
                    .py(px(6.0))
                    .border_t_1()
                    .border_color(t.border)
                    .bg(t.surface_1)
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(t.fs_xs())
                    .text_color(t.text_3)
                    .child("↑↓ 选择 · ⏎ 执行 · Esc 关闭")
                    .child("⌘K"),
            )
            .into_any_element()
    }

    fn modal_conn_manager(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let active_id = self
            .data
            .active_conn
            .as_ref()
            .map(|c| c.id.as_str())
            .unwrap_or("");
        let body = div()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .when(self.data.connections.is_empty(), |d| {
                d.child(
                    div()
                        .py(px(24.0))
                        .text_center()
                        .text_size(t.fs_md())
                        .text_color(t.text_3)
                        .child("暂无已保存的连接"),
                )
            })
            .children(self.data.connections.iter().enumerate().map(|(i, c)| {
                let c_color = parse_hex(&c.color).unwrap_or(t.accent);
                let is_active = c.id == active_id;
                let connect_id = c.id.clone();
                let edit_id = c.id.clone();
                let del_id = c.id.clone();
                let edit_tip_theme = *t;
                let del_tip_theme = *t;
                div()
                    .id(("cm-item", i))
                    .p(px(10.0))
                    .rounded(px(9.0))
                    .border_1()
                    .border_color(if is_active { t.accent } else { t.border })
                    .bg(if is_active {
                        t.accent_soft
                    } else {
                        t.surface_1
                    })
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .cursor_pointer()
                    .hover(|s| s.border_color(t.accent))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(cfg) = this.data.connections.iter().find(|c| c.id == connect_id).cloned() {
                            this.connect_with_config(cfg, cx);
                        }
                    }))
                    .child(
                        div()
                            .w(px(28.0))
                            .h(px(28.0))
                            .rounded(px(7.0))
                            .bg(c_color)
                            .shadow_sm()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(db_driver_icon(c.driver, 16.0, true)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(t.fs_md())
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(c.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(t.fs_sm())
                                    .text_color(t.text_3)
                                    .child(format!(
                                        "{} · {}",
                                        c.driver.display_name(),
                                        c.display_host()
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(with_alpha(c_color, 0.14))
                            .text_color(c_color)
                            .text_size(t.fs_xs())
                            .child(c.label.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .id(("cm-edit-conn", i))
                                    .w(px(26.0))
                                    .h(px(26.0))
                                    .rounded(px(6.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.surface_2))
                                    .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                                    .tooltip(move |window, cx| {
                                        build_sidebar_tooltip(
                                            "编辑连接".to_string(),
                                            edit_tip_theme,
                                            window,
                                            cx,
                                        )
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        if let Some(cfg) = this.data.connections.iter().find(|c| c.id == edit_id).cloned() {
                                            this.open_edit_connection_modal(cfg, true, window, cx);
                                        }
                                    }))
                                    .child(hgi("edit", 13.0, t.text_2)),
                            )
                            .child(
                                div()
                                    .id(("cm-del-conn", i))
                                    .w(px(26.0))
                                    .h(px(26.0))
                                    .rounded(px(6.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.red_soft))
                                    .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                                    .tooltip(move |window, cx| {
                                        build_sidebar_tooltip(
                                            "删除连接".to_string(),
                                            del_tip_theme,
                                            window,
                                            cx,
                                        )
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.data.connections.retain(|x| x.id != del_id);
                                        save_connections(&this.data.connections);
                                        this.maybe_auto_push_sync(cx);
                                        this.show_toast("已删除连接", cx);
                                    }))
                                    .child(hgi("trash", 13.0, t.text_3)),
                            ),
                    )
            }))
            .into_any_element();

        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .child(
                div()
                    .id("cm-disconnect-btn")
                    .h(px(28.0))
                    .px(px(10.0))
                    .rounded(px(6.0))
                    .bg(t.red_soft)
                    .text_color(t.red)
                    .text_size(t.fs_md())
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.disconnect(cx);
                    }))
                    .child("断开连接"),
            )
            .child(
                div()
                    .id("cm-new-btn")
                    .h(px(28.0))
                    .px(px(12.0))
                    .rounded(px(6.0))
                    .bg(t.accent)
                    .text_color(gpui::white())
                    .text_size(t.fs_md())
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_new_connection_modal(window, cx);
                    }))
                    .child(hgi("plus", 14.0, gpui::white()))
                    .child("新建连接"),
            )
            .into_any_element();

        self.modal_shell(
            "conn-manager-modal",
            560.0,
            "数据库连接管理".into(),
            body,
            Some(footer),
            t,
            cx,
        )
    }

    fn modal_new_conn(&self, step: u8, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        if step == 1 {
            let drivers = [
                DbDriver::Postgres,
                DbDriver::Mysql,
                DbDriver::Sqlite,
                DbDriver::Redis,
                DbDriver::Clickhouse,
                DbDriver::Mariadb,
            ];
            let body = div()
                .p(px(16.0))
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .text_size(t.fs_md())
                        .text_color(t.text_3)
                        .child("请选择要连接的数据库引擎（当前已支持 PostgreSQL 与 MySQL）"),
                )
                .child(
                    div()
                        .grid()
                        .grid_cols(3)
                        .gap(px(10.0))
                        .children(drivers.into_iter().enumerate().map(|(i, drv)| {
                            let supported = drv.is_supported();
                            div()
                                .id(("drv-card", i))
                                .p(px(16.0))
                                .rounded(px(9.0))
                                .bg(t.surface_1)
                                .border_1()
                                .border_color(t.border)
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap(px(8.0))
                                .when(supported, |d| {
                                    d.cursor_pointer()
                                        .hover(|s| s.border_color(t.accent).bg(t.panel_bg))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.data.new_conn.driver = drv;
                                            this.data.new_conn.port =
                                                drv.default_port().to_string();
                                            this.data.new_conn.database =
                                                drv.default_db().to_string();
                                            this.data.new_conn.username =
                                                drv.default_user().to_string();
                                            this.data.new_conn.name = format!(
                                                "local_{}",
                                                drv.display_name().to_lowercase()
                                            );
                                            this.sync_new_conn_inputs(window, cx);
                                            this.data.active_modal =
                                                ActiveModal::NewConnection { step: 2 };
                                            cx.notify();
                                        }))
                                })
                                .when(!supported, |d| d.opacity(0.45).cursor_not_allowed())
                                .child(
                                    div()
                                        .w(px(40.0))
                                        .h(px(40.0))
                                        .rounded(px(10.0))
                                        .bg(if supported { t.panel_bg } else { t.surface_2 })
                                        .border_1()
                                        .border_color(t.border)
                                        .when(supported, |d| d.shadow_sm())
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(db_driver_icon(drv, 26.0, false)),
                                )
                                .when(!supported, |d| {
                                    d.child(
                                        div()
                                            .px(px(6.0))
                                            .py(px(1.5))
                                            .rounded(px(4.0))
                                            .bg(t.surface_2)
                                            .border_1()
                                            .border_color(t.border)
                                            .text_size(t.fs_xs())
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(t.text_3)
                                            .child("即将支持"),
                                    )
                                })
                                .child(
                                    div()
                                        .text_size(t.fs_lg())
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(if supported { t.text_1 } else { t.text_3 })
                                        .child(drv.display_name()),
                                )
                                .when(supported, |d| {
                                    d.child(
                                        div()
                                            .text_size(t.fs_xs())
                                            .text_color(t.green)
                                            .child(format!("端口 {}", drv.default_port())),
                                    )
                                })
                        })),
                )
                .into_any_element();

            return self.modal_shell(
                "new-conn-step1",
                600.0,
                "选择数据库类型".into(),
                body,
                None,
                t,
                cx,
            );
        }

        let swatches = [
            "#ef4444", "#f59e0b", "#10b981", "#3b82f6", "#6366f1", "#8b5cf6", "#06b6d4",
        ];
        let nc = &self.data.new_conn;
        let is_editing = nc.editing_id.is_some();

        let field_box = |label: &'static str, input_el: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(t.fs_sm())
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(t.text_2)
                        .child(label),
                )
                .child(
                    div()
                        .px(px(8.0))
                        .py(px(2.0))
                        .rounded(px(6.0))
                        .bg(t.surface_1)
                        .border_1()
                        .border_color(t.border)
                        .child(input_el),
                )
        };

        let supported_drivers = [DbDriver::Postgres, DbDriver::Mysql];
        let body = div()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .when(is_editing, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_size(t.fs_sm())
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(t.text_2)
                                .child("数据库类型"),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .children(supported_drivers.into_iter().enumerate().map(
                                    |(i, drv)| {
                                        let active = nc.driver == drv;
                                        div()
                                            .id(("edit-drv-pill", i))
                                            .h(px(30.0))
                                            .px(px(10.0))
                                            .rounded(px(7.0))
                                            .bg(if active { t.accent_soft } else { t.surface_1 })
                                            .border_1()
                                            .border_color(if active { t.accent } else { t.border })
                                            .text_size(t.fs_md())
                                            .font_weight(if active {
                                                gpui::FontWeight::SEMIBOLD
                                            } else {
                                                gpui::FontWeight::NORMAL
                                            })
                                            .text_color(if active { t.accent } else { t.text_2 })
                                            .flex()
                                            .items_center()
                                            .gap(px(6.0))
                                            .cursor_pointer()
                                            .hover(|s| s.border_color(t.accent))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                let old_drv = this.data.new_conn.driver;
                                                if old_drv == drv {
                                                    return;
                                                }
                                                this.data.new_conn.driver = drv;
                                                let cur_port = this
                                                    .nc_port_input
                                                    .read(cx)
                                                    .value()
                                                    .trim()
                                                    .to_string();
                                                if cur_port.is_empty()
                                                    || cur_port
                                                        == old_drv.default_port().to_string()
                                                {
                                                    let next_port = drv.default_port().to_string();
                                                    this.data.new_conn.port = next_port.clone();
                                                    this.nc_port_input.update(cx, |s, cx| {
                                                        s.set_value(next_port, window, cx)
                                                    });
                                                }
                                                cx.notify();
                                            }))
                                            .child(db_driver_icon(drv, 15.0, false))
                                            .child(drv.display_name())
                                    },
                                )),
                        ),
                )
            })
            .child(field_box(
                "快捷粘贴连接 URI（可选）",
                Input::new(&self.nc_uri_input).appearance(false).px_0().py_0().h(t.control_h()).text_size(t.fs_md()).into_any_element(),
            ))
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(px(10.0))
                    .child(field_box(
                        "连接名称",
                        Input::new(&self.nc_name_input).appearance(false).px_0().py_0().h(t.control_h()).text_size(t.fs_md()).into_any_element(),
                    ))
                    .child(field_box(
                        "环境标签（自定义）",
                        Input::new(&self.nc_tag_input).appearance(false).px_0().py_0().h(t.control_h()).text_size(t.fs_md()).into_any_element(),
                    )),
            )
            // Color picker bar
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(t.fs_sm())
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text_2)
                            .child("连接主色（应用于首页图标与工作台主题色）"),
                    )
                    .child(
                        div()
                            .px(px(12.0))
                            .py(px(8.0))
                            .rounded(px(9.0))
                            .bg(t.surface_1)
                            .border_1()
                            .border_color(t.border)
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.0))
                                    .children(swatches.into_iter().enumerate().map(|(i, hex)| {
                                        let c = parse_hex(hex).unwrap_or(t.accent);
                                        let active = nc.color.eq_ignore_ascii_case(hex);
                                        div()
                                            .id(("nc-swatch", i))
                                            .w(px(20.0))
                                            .h(px(20.0))
                                            .rounded_full()
                                            .bg(c)
                                            .cursor_pointer()
                                            .border_2()
                                            .border_color(if active {
                                                t.text_1
                                            } else {
                                                gpui::transparent_black()
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.data.new_conn.color = hex.to_string();
                                                cx.notify();
                                            }))
                                    })),
                            )
                            .child(
                                div()
                                    .text_size(t.fs_sm())
                                    .text_color(t.text_2)
                                    .child(nc.color.to_uppercase()),
                            ),
                    ),
            )
            // Network fields
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(px(10.0))
                    .child(field_box(
                        "主机地址",
                        Input::new(&self.nc_host_input).appearance(false).px_0().py_0().h(t.control_h()).text_size(t.fs_md()).into_any_element(),
                    ))
                    .child(field_box(
                        "端口",
                        Input::new(&self.nc_port_input).appearance(false).px_0().py_0().h(t.control_h()).text_size(t.fs_md()).into_any_element(),
                    ))
                    .child(field_box(
                        "数据库名",
                        Input::new(&self.nc_db_input).appearance(false).px_0().py_0().h(t.control_h()).text_size(t.fs_md()).into_any_element(),
                    ))
                    .child(field_box(
                        "用户名",
                        Input::new(&self.nc_user_input).appearance(false).px_0().py_0().h(t.control_h()).text_size(t.fs_md()).into_any_element(),
                    ))
                    .child(field_box(
                        "密码",
                        Input::new(&self.nc_pass_input).appearance(false).px_0().py_0().h(t.control_h()).text_size(t.fs_md()).into_any_element(),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .text_size(t.fs_sm())
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(t.text_2)
                                    .child("SSL 模式"),
                            )
                            .child(self.select_dropdown(
                                "nc-ssl",
                                nc.ssl_mode.as_str().to_string(),
                                true,
                                vec![
                                    (
                                        "disable",
                                        nc.ssl_mode == SslMode::Disable,
                                        Box::new(cx.listener(|this, _, _, cx| {
                                            this.data.new_conn.ssl_mode = SslMode::Disable;
                                            this.data.open_dropdown = None;
                                            cx.notify();
                                        })),
                                    ),
                                    (
                                        "prefer",
                                        nc.ssl_mode == SslMode::Prefer,
                                        Box::new(cx.listener(|this, _, _, cx| {
                                            this.data.new_conn.ssl_mode = SslMode::Prefer;
                                            this.data.open_dropdown = None;
                                            cx.notify();
                                        })),
                                    ),
                                    (
                                        "require",
                                        nc.ssl_mode == SslMode::Require,
                                        Box::new(cx.listener(|this, _, _, cx| {
                                            this.data.new_conn.ssl_mode = SslMode::Require;
                                            this.data.open_dropdown = None;
                                            cx.notify();
                                        })),
                                    ),
                                    (
                                        "verify-full",
                                        nc.ssl_mode == SslMode::VerifyFull,
                                        Box::new(cx.listener(|this, _, _, cx| {
                                            this.data.new_conn.ssl_mode = SslMode::VerifyFull;
                                            this.data.open_dropdown = None;
                                            cx.notify();
                                        })),
                                    ),
                                ],
                                t,
                                cx,
                            )),
                    ),
            )
            // SSH Tunnel Toggle
            .child(
                div()
                    .rounded(px(9.0))
                    .bg(t.surface_1)
                    .border_1()
                    .border_color(t.border)
                    .overflow_hidden()
                    .child(
                        div()
                            .px(px(12.0))
                            .py(px(10.0))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_size(t.fs_md())
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child("SSH 跳板机转发 (SSH Tunnel)"),
                                    )
                                    .child(
                                        div()
                                            .text_size(t.fs_sm())
                                            .text_color(t.text_3)
                                            .child("通过 Bastion 跳板机加密转发内网数据库端口"),
                                    ),
                            )
                            .child(self.toggle_pill(
                                "nc-ssh-toggle",
                                nc.ssh_enabled,
                                t,
                                cx.listener(|this, _, _, cx| {
                                    this.data.new_conn.ssh_enabled =
                                        !this.data.new_conn.ssh_enabled;
                                    cx.notify();
                                }),
                            )),
                    )
                    .when(nc.ssh_enabled, |d| {
                        d.child(
                            div()
                                .p(px(12.0))
                                .border_t_1()
                                .border_color(t.border)
                                .grid()
                                .grid_cols(2)
                                .gap(px(10.0))
                                .child(field_box(
                                    "跳板机地址 (SSH Host)",
                                    Input::new(&self.nc_ssh_host_input)
                                        .appearance(false)
                                        .px_0()
                                        .py_0()
                                        .h(t.control_h())
                                        .text_size(t.fs_md())
                                        .into_any_element(),
                                ))
                                .child(field_box(
                                    "SSH 端口",
                                    Input::new(&self.nc_ssh_port_input)
                                        .appearance(false)
                                        .px_0()
                                        .py_0()
                                        .h(t.control_h())
                                        .text_size(t.fs_md())
                                        .into_any_element(),
                                ))
                                .child(field_box(
                                    "SSH 用户名",
                                    Input::new(&self.nc_ssh_user_input)
                                        .appearance(false)
                                        .px_0()
                                        .py_0()
                                        .h(t.control_h())
                                        .text_size(t.fs_md())
                                        .into_any_element(),
                                ))
                                .child(field_box(
                                    "私钥路径",
                                    Input::new(&self.nc_ssh_key_input)
                                        .appearance(false)
                                        .px_0()
                                        .py_0()
                                        .h(t.control_h())
                                        .text_size(t.fs_md())
                                        .into_any_element(),
                                )),
                        )
                    }),
            )
            // Read-Only lock
            .child(
                div()
                    .px(px(12.0))
                    .py(px(9.0))
                    .rounded(px(9.0))
                    .bg(t.surface_1)
                    .border_1()
                    .border_color(t.border)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(t.fs_md())
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child("只读安全锁（禁止写入与 DDL 变更）"),
                    )
                    .child(self.toggle_pill(
                        "nc-ro-toggle",
                        nc.read_only,
                        t,
                        cx.listener(|this, _, _, cx| {
                            this.data.new_conn.read_only = !this.data.new_conn.read_only;
                            cx.notify();
                        }),
                    )),
            )
            .into_any_element();

        let footer_left: AnyElement = if is_editing {
            div()
                .id("nc-cancel-edit")
                .h(px(28.0))
                .px(px(10.0))
                .rounded(px(6.0))
                .bg(t.surface_2)
                .border_1()
                .border_color(t.border)
                .text_size(t.fs_md())
                .flex()
                .items_center()
                .cursor_pointer()
                .hover(|s| s.bg(t.surface_3))
                .on_click(cx.listener(|this, _, _, cx| {
                    let return_to_manager = this.data.new_conn.return_to_manager;
                    this.data.new_conn.editing_id = None;
                    this.data.new_conn.return_to_manager = false;
                    this.data.open_dropdown = None;
                    this.data.active_modal = if return_to_manager {
                        ActiveModal::ConnectionManager
                    } else {
                        ActiveModal::None
                    };
                    cx.notify();
                }))
                .child("取消")
                .into_any_element()
        } else {
            div()
                .id("nc-back-step1")
                .h(px(28.0))
                .px(px(10.0))
                .rounded(px(6.0))
                .bg(t.surface_2)
                .border_1()
                .border_color(t.border)
                .text_size(t.fs_md())
                .flex()
                .items_center()
                .gap(px(5.0))
                .cursor_pointer()
                .hover(|s| s.bg(t.surface_3))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.data.active_modal = ActiveModal::NewConnection { step: 1 };
                    cx.notify();
                }))
                .child(hgi("arrow-left", 13.0, t.text_2))
                .child("返回选择类型")
                .into_any_element()
        };

        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .child(footer_left)
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("nc-test-btn")
                            .h(px(28.0))
                            .px(px(10.0))
                            .rounded(px(6.0))
                            .bg(t.surface_2)
                            .border_1()
                            .border_color(t.border)
                            .text_size(t.fs_md())
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(t.surface_3))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.test_new_conn(cx);
                            }))
                            .child("测试连接"),
                    )
                    .child(
                        div()
                            .id("nc-save-only-btn")
                            .h(px(28.0))
                            .px(px(10.0))
                            .rounded(px(6.0))
                            .bg(t.surface_2)
                            .border_1()
                            .border_color(t.border)
                            .text_size(t.fs_md())
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(t.surface_3))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_conn_from_form(false, cx);
                            }))
                            .child("保存"),
                    )
                    .child(
                        div()
                            .id("nc-save-connect-btn")
                            .h(px(28.0))
                            .px(px(12.0))
                            .rounded(px(6.0))
                            .bg(t.accent)
                            .text_color(gpui::white())
                            .text_size(t.fs_md())
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.92))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_conn_from_form(true, cx);
                            }))
                            .child("保存并连接"),
                    ),
            )
            .into_any_element();

        self.modal_shell(
            "new-conn-step2",
            600.0,
            if is_editing {
                format!("编辑 {} 连接", nc.driver.display_name())
            } else {
                format!("新建 {} 连接", nc.driver.display_name())
            },
            body,
            Some(footer),
            t,
            cx,
        )
    }

    fn modal_mcp(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let conn_name = self
            .data
            .active_conn
            .as_ref()
            .map(|c| c.name.as_str())
            .unwrap_or("local_postgres_dev");
        let ro_flag = if self.data.settings.mcp_read_only {
            ", \"--read-only\""
        } else {
            ""
        };
        let json_cfg = format!(
            "{{\n  \"mcpServers\": {{\n    \"velodb\": {{\n      \"command\": \"velodb-mcp\",\n      \"args\": [\"--connection\", \"{conn_name}\"{ro_flag}]\n    }}\n  }}\n}}"
        );
        let json_copy = json_cfg.clone();

        let body = div()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .text_size(t.fs_md())
                    .text_color(t.text_2)
                    .child("VeloDB 将当前数据库连接与 Schema 能力通过标准 MCP (Model Context Protocol) 暴露给外部 Agent（Claude Code / Cursor / Antigravity）。"),
            )
            .child(
                div()
                    .p(px(12.0))
                    .rounded(px(9.0))
                    .bg(t.surface_1)
                    .border_1()
                    .border_color(t.border)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(t.fs_md())
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Agent 只读安全锁"),
                            )
                            .child(
                                div()
                                    .text_size(t.fs_sm())
                                    .text_color(t.text_3)
                                    .child("暴露 list_schemas / describe_table / run_read_query / explain_query"),
                            ),
                    )
                    .child(
                        div()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(t.green_soft)
                            .text_color(t.green)
                            .text_size(t.fs_xs())
                            .child(if self.data.settings.mcp_read_only {
                                "已启用"
                            } else {
                                "读写"
                            }),
                    ),
            )
            .child(
                div()
                    .p(px(12.0))
                    .rounded(px(6.0))
                    .bg(t.surface_1)
                    .border_1()
                    .border_color(t.border)
                    .text_size(t.fs_sm())
                    .child(json_cfg),
            )
            .into_any_element();

        let footer = div()
            .flex()
            .gap(px(8.0))
            .child(
                div()
                    .id("mcp-copy-btn")
                    .h(px(28.0))
                    .px(px(12.0))
                    .rounded(px(6.0))
                    .bg(t.accent)
                    .text_color(gpui::white())
                    .text_size(t.fs_md())
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(json_copy.clone()));
                        this.data.active_modal = ActiveModal::None;
                        this.show_toast("已复制 MCP 配置 JSON", cx);
                    }))
                    .child("复制配置 JSON"),
            )
            .into_any_element();

        self.modal_shell(
            "mcp-modal",
            560.0,
            "🔌 VeloDB 本地 MCP 服务".into(),
            body,
            Some(footer),
            t,
            cx,
        )
    }

    fn modal_columns_picker(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let Some(ts) = self.active_tab().and_then(|tab| tab.table_state.as_ref()) else {
            return div().into_any_element();
        };
        let body = div()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .children(ts.columns.iter().enumerate().map(|(i, c)| {
                let visible = !ts.hidden_cols.contains(&c.name);
                let col_name = c.name.clone();
                div()
                    .id(("col-vis-row", i))
                    .flex()
                    .items_center()
                    .justify_between()
                    .p(px(6.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.surface_1))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(ts) =
                            this.active_tab_mut().and_then(|t| t.table_state.as_mut())
                        {
                            if ts.hidden_cols.contains(&col_name) {
                                ts.hidden_cols.remove(&col_name);
                            } else {
                                ts.hidden_cols.insert(col_name.clone());
                            }
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(if visible { "☑" } else { "☐" })
                            .child(c.name.clone()),
                    )
                    .child(
                        div()
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .bg(t.surface_2)
                            .text_size(t.fs_xs())
                            .text_color(t.text_3)
                            .child(c.data_type.clone()),
                    )
            }))
            .into_any_element();

        self.modal_shell("cols-modal", 360.0, "列显示配置".into(), body, None, t, cx)
    }

    fn modal_preview_sql(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let is_mysql = self
            .data
            .active_conn
            .as_ref()
            .map(|c| c.driver == DbDriver::Mysql)
            .unwrap_or(false);
        let sql_text = if is_mysql {
            generate_mysql_staged_sql(&self.data.staged_changes)
        } else {
            generate_staged_sql(&self.data.staged_changes)
        };
        let is_prod = self
            .data
            .active_conn
            .as_ref()
            .map(|c| c.env == "prod")
            .unwrap_or(false);

        let body = div()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .when(self.data.settings.safe_mode && is_prod, |d| {
                d.child(
                    div()
                        .px(px(10.0))
                        .py(px(8.0))
                        .rounded(px(6.0))
                        .bg(t.red_soft)
                        .text_color(t.red)
                        .text_size(t.fs_md())
                        .child("⚠ 安全模式：当前连接为生产环境，请仔细核对事务 SQL。"),
                )
            })
            .child(
                div()
                    .p(px(12.0))
                    .rounded(px(6.0))
                    .bg(t.surface_1)
                    .border_1()
                    .border_color(t.border)
                    .text_size(t.fs_sm())
                    .child(sql_text),
            )
            .into_any_element();

        let footer = div()
            .flex()
            .gap(px(8.0))
            .child(
                div()
                    .id("confirm-commit-btn")
                    .h(px(28.0))
                    .px(px(12.0))
                    .rounded(px(6.0))
                    .bg(t.green)
                    .text_color(gpui::rgb(0x06281e))
                    .text_size(t.fs_md())
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.commit_staged_changes(cx);
                    }))
                    .child("确认执行 COMMIT"),
            )
            .into_any_element();

        self.modal_shell(
            "preview-sql-modal",
            580.0,
            "预览事务 SQL".into(),
            body,
            Some(footer),
            t,
            cx,
        )
    }

    fn modal_export_import(
        &self,
        is_import: bool,
        t: &VeloTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if is_import {
            let body = div()
                .p(px(16.0))
                .child(
                    div()
                        .id("import-dropzone")
                        .p(px(28.0))
                        .rounded(px(9.0))
                        .border_1()
                        .border_color(t.border_strong)
                        .bg(t.surface_1)
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(8.0))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_toast("请将 CSV / JSON 文件拖入窗口或粘贴数据导入", cx);
                        }))
                        .child(
                            div()
                                .text_size(t.fs_lg())
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("拖拽 CSV / JSON 文件到此处"),
                        )
                        .child(
                            div()
                                .text_size(t.fs_sm())
                                .text_color(t.text_3)
                                .child("支持自动推断列名、NULL 映射与冲突策略 (ON CONFLICT DO NOTHING)"),
                        ),
                )
                .into_any_element();
            return self.modal_shell(
                "import-modal",
                560.0,
                "导入 CSV / JSON 到数据表".into(),
                body,
                None,
                t,
                cx,
            );
        }

        let preview = if let Some(ts) = self.active_tab().and_then(|tab| tab.table_state.as_ref()) {
            let cols: Vec<String> = ts.columns.iter().map(|c| c.name.clone()).collect();
            let sample_rows: Vec<Vec<Option<String>>> = ts.rows.iter().take(20).cloned().collect();
            export_rows(&cols, &sample_rows, &ts.schema, &ts.table, self.data.export_fmt)
        } else {
            String::new()
        };
        let preview_copy = preview.clone();

        let fmt_btn = |id: &'static str, label: &'static str, fmt: ExportFormat| {
            let active = self.data.export_fmt == fmt;
            div()
                .id(id)
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(4.0))
                .text_size(t.fs_sm())
                .cursor_pointer()
                .bg(if active {
                    t.panel_bg
                } else {
                    gpui::transparent_black()
                })
                .text_color(if active { t.text_1 } else { t.text_2 })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.data.export_fmt = fmt;
                    cx.notify();
                }))
                .child(label)
        };

        let body = div()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .self_start()
                    .p(px(2.0))
                    .rounded(px(6.0))
                    .bg(t.surface_2)
                    .border_1()
                    .border_color(t.border)
                    .child(fmt_btn("exp-csv", "CSV", ExportFormat::Csv))
                    .child(fmt_btn("exp-json", "JSON", ExportFormat::Json))
                    .child(fmt_btn("exp-sql", "SQL INSERT", ExportFormat::SqlInsert)),
            )
            .child(
                div()
                    .p(px(12.0))
                    .rounded(px(6.0))
                    .bg(t.surface_1)
                    .border_1()
                    .border_color(t.border)
                    .max_h(px(240.0))
                    .overflow_y_scrollbar()
                    .text_size(t.fs_sm())
                    .child(preview),
            )
            .into_any_element();

        let footer = div()
            .flex()
            .gap(px(8.0))
            .child(
                div()
                    .id("copy-export-btn")
                    .h(px(28.0))
                    .px(px(12.0))
                    .rounded(px(6.0))
                    .bg(t.accent)
                    .text_color(gpui::white())
                    .text_size(t.fs_md())
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(preview_copy.clone()));
                        this.data.active_modal = ActiveModal::None;
                        this.show_toast("已复制导出内容", cx);
                    }))
                    .child("复制输出内容"),
            )
            .into_any_element();

        self.modal_shell(
            "export-modal",
            580.0,
            "导出表数据".into(),
            body,
            Some(footer),
            t,
            cx,
        )
    }

    fn modal_snippets(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let body = div()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .px(px(8.0))
                            .rounded(px(6.0))
                            .bg(t.surface_1)
                            .border_1()
                            .border_color(t.border)
                            .child(
                                Input::new(&self.snippet_name_input)
                                    .appearance(false)
                                    .px_0()
                                    .py_0()
                                    .h(t.control_h())
                                    .text_size(t.fs_md()),
                            ),
                    )
                    .child(
                        div()
                            .id("save-new-snip-btn")
                            .h(px(30.0))
                            .px(px(12.0))
                            .rounded(px(6.0))
                            .bg(t.accent)
                            .text_color(gpui::white())
                            .text_size(t.fs_md())
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                let name = this.snippet_name_input.read(cx).value().trim().to_string();
                                let Some(editor) = this.active_sql_editor() else { return };
                                let value = editor.read(cx).value();
                                let sql = value.trim();
                                if name.is_empty() || sql.is_empty() {
                                    this.show_toast("请输入片段名称", cx);
                                    return;
                                }
                                this.data.snippets.insert(
                                    0,
                                    SqlSnippet {
                                        id: format!("snip-{}", uuid::Uuid::new_v4().simple()),
                                        name: name.clone(),
                                        sql: sql.into(),
                                    },
                                );
                                save_snippets(&this.data.snippets);
                                this.snippet_name_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.show_toast(format!("已保存片段：{name}"), cx);
                            }))
                            .child("保存当前 SQL"),
                    ),
            )
            .children(self.data.snippets.iter().enumerate().map(|(i, s)| {
                let sql_c = s.sql.clone();
                let name_c = s.name.clone();
                let id_c = s.id.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .p(px(9.0))
                    .rounded(px(6.0))
                    .bg(t.surface_1)
                    .border_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(t.fs_md())
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(s.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(t.fs_sm())
                                    .text_color(t.text_3)
                                    .truncate()
                                    .child(super::sql_workbench::sql_preview(&s.sql)),
                            ),
                    )
                    .child(
                        div()
                            .id(("load-snip", i))
                            .h(px(24.0))
                            .px(px(8.0))
                            .rounded(px(5.0))
                            .bg(t.surface_2)
                            .text_size(t.fs_sm())
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let Some(editor) = this.active_sql_editor() else { return };
                                editor.update(cx, |st, cx| st.set_value(sql_c.to_string(), window, cx));
                                this.data.active_modal = ActiveModal::None;
                                this.show_toast(format!("已载入片段：{name_c}"), cx);
                            }))
                            .child("载入"),
                    )
                    .child(
                        div()
                            .id(("del-snip", i))
                            .w(px(24.0))
                            .h(px(24.0))
                            .rounded(px(5.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(t.red_soft))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.data.snippets.retain(|x| x.id != id_c);
                                save_snippets(&this.data.snippets);
                                cx.notify();
                            }))
                            .child(hgi("trash", 13.0, t.red)),
                    )
            }))
            .into_any_element();

        self.modal_shell(
            "snippets-modal",
            560.0,
            "管理 SQL 代码片段".into(),
            body,
            None,
            t,
            cx,
        )
    }

    fn modal_settings(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let cat = self.data.settings_cat;
        let s = &self.data.settings;

        let nav_items: [(SettingsCategory, u32, u32, &'static str, &'static str); 8] = [
            (SettingsCategory::General, 0x4da3ff, 0x0a7aff, "paint", "常规与外观"),
            (SettingsCategory::Grid, 0x7c83ff, 0x5856d6, "table", "数据网格与格式"),
            (SettingsCategory::Editor, 0xc084fc, 0x9333ea, "code", "SQL 编辑器"),
            (SettingsCategory::Safety, 0x4ade80, 0x16a34a, "shield-check", "安全与查询"),
            (SettingsCategory::Keys, 0xfb923c, 0xea580c, "keyboard", "快捷键"),
            (SettingsCategory::Mcp, 0x2dd4bf, 0x0d9488, "plug", "MCP 服务"),
            (SettingsCategory::Sync, 0x38bdf8, 0x0284c7, "cloud", "云端同步"),
            (SettingsCategory::Advanced, 0x94a3b8, 0x475569, "sliders", "高级与配置"),
        ];

        let row = |label: &'static str, desc: &'static str, ctrl: AnyElement| {
            div()
                .px(px(14.0))
                .py(px(11.0))
                .border_b_1()
                .border_color(t.border)
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(t.fs_md())
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(label),
                        )
                        .child(div().text_size(t.fs_sm()).text_color(t.text_3).child(desc)),
                )
                .child(ctrl)
        };

        let pane_body: AnyElement = match cat {
            SettingsCategory::General => div()
                .rounded(px(9.0))
                .bg(t.surface_1)
                .border_1()
                .border_color(t.border)
                .child(row(
                    "界面主题",
                    "默认跟随系统，可手动指定浅色或深色",
                    self.select_dropdown(
                        "set-theme",
                        match s.theme.as_str() {
                            "light" => "浅色",
                            "dark" => "深色",
                            _ => "跟随系统",
                        }
                        .into(),
                        false,
                        vec![
                            (
                                "跟随系统",
                                s.theme == "system" || (s.theme != "light" && s.theme != "dark"),
                                Box::new(cx.listener(|this, _, window, cx| {
                                    this.data.settings.theme = "system".into();
                                    this.data.open_dropdown = None;
                                    this.sync_component_theme(window, cx);
                                    save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                                    cx.notify();
                                })),
                            ),
                            (
                                "浅色",
                                s.theme == "light",
                                Box::new(cx.listener(|this, _, window, cx| {
                                    this.data.settings.theme = "light".into();
                                    this.data.open_dropdown = None;
                                    this.sync_component_theme(window, cx);
                                    save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                                    cx.notify();
                                })),
                            ),
                            (
                                "深色",
                                s.theme == "dark",
                                Box::new(cx.listener(|this, _, window, cx| {
                                    this.data.settings.theme = "dark".into();
                                    this.data.open_dropdown = None;
                                    this.sync_component_theme(window, cx);
                                    save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                                    cx.notify();
                                })),
                            ),
                        ],
                        t,
                        cx,
                    ),
                ))
                .child(row(
                    "macOS 毛玻璃透明效果",
                    "开启窗口与侧边栏毛玻璃半透明质感",
                    self.toggle_pill(
                        "set-vibrancy",
                        s.vibrancy,
                        t,
                        cx.listener(|this, _, _, cx| {
                            this.data.settings.vibrancy = !this.data.settings.vibrancy;
                            save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                            cx.notify();
                        }),
                    ),
                ))
                .child(row(
                    "表格行高密度",
                    "控制数据网格的行高紧凑度",
                    self.select_dropdown(
                        "set-density",
                        match s.row_density.as_str() {
                            "compact" => "紧凑",
                            "spacious" => "宽松",
                            _ => "舒适",
                        }
                        .into(),
                        false,
                        vec![
                            (
                                "紧凑",
                                s.row_density == "compact",
                                Box::new(cx.listener(|this, _, _, cx| {
                                    this.data.settings.row_density = "compact".into();
                                    this.data.open_dropdown = None;
                                    save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                                    cx.notify();
                                })),
                            ),
                            (
                                "舒适",
                                s.row_density != "compact" && s.row_density != "spacious",
                                Box::new(cx.listener(|this, _, _, cx| {
                                    this.data.settings.row_density = "comfortable".into();
                                    this.data.open_dropdown = None;
                                    save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                                    cx.notify();
                                })),
                            ),
                            (
                                "宽松",
                                s.row_density == "spacious",
                                Box::new(cx.listener(|this, _, _, cx| {
                                    this.data.settings.row_density = "spacious".into();
                                    this.data.open_dropdown = None;
                                    save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                                    cx.notify();
                                })),
                            ),
                        ],
                        t,
                        cx,
                    ),
                ))
                .child(row(
                    "全局字号",
                    "统一调整侧边栏、输入框、表格、右侧检视器与编辑器字号 (11px – 16px)",
                    self.select_dropdown(
                        "set-global-fs",
                        format!("{} px", s.font_size),
                        false,
                        [11u16, 12, 13, 14, 15, 16]
                            .into_iter()
                            .map(|sz| {
                                let label: &'static str = match sz {
                                    11 => "11 px",
                                    12 => "12 px",
                                    13 => "13 px",
                                    14 => "14 px",
                                    15 => "15 px",
                                    _ => "16 px",
                                };
                                (
                                    label,
                                    s.font_size == sz,
                                    Box::new(cx.listener(move |this, _, window, cx| {
                                        this.data.settings.set_global_font_size(sz);
                                        this.data.open_dropdown = None;
                                        this.sync_component_theme(window, cx);
                                        save_settings(&this.data.settings);
                                        this.maybe_auto_push_sync(cx);
                                        cx.notify();
                                    })) as Box<dyn Fn(&gpui::MouseDownEvent, &mut gpui::Window, &mut gpui::App)>,
                                )
                            })
                            .collect(),
                        t,
                        cx,
                    ),
                ))
                .into_any_element(),

            SettingsCategory::Grid => div()
                .rounded(px(9.0))
                .bg(t.surface_1)
                .border_1()
                .border_color(t.border)
                .child(row(
                    "默认分页 LIMIT",
                    "打开表时默认拉取的行数",
                    self.select_dropdown(
                        "set-limit",
                        format!("{} 行", s.default_limit),
                        false,
                        [(50usize, "50 行"), (100, "100 行"), (500, "500 行"), (1000, "1000 行")]
                            .into_iter()
                            .map(|(lim, label)| {
                                (
                                    label,
                                    s.default_limit == lim,
                                    Box::new(cx.listener(move |this, _, _, cx| {
                                        this.data.settings.default_limit = lim;
                                        this.data.open_dropdown = None;
                                        save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                                        cx.notify();
                                    })) as Box<dyn Fn(&gpui::MouseDownEvent, &mut gpui::Window, &mut gpui::App)>,
                                )
                            })
                            .collect(),
                        t,
                        cx,
                    ),
                ))
                .child(row(
                    "NULL 空值显示",
                    "单元格为空值时的占位文案",
                    self.select_dropdown(
                        "set-null-lbl",
                        s.null_label.clone(),
                        false,
                        ["NULL", "∅", "(null)"]
                            .into_iter()
                            .map(|lbl| {
                                (
                                    lbl,
                                    s.null_label == lbl,
                                    Box::new(cx.listener(move |this, _, _, cx| {
                                        this.data.settings.null_label = lbl.into();
                                        this.data.open_dropdown = None;
                                        save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                                        cx.notify();
                                    })) as Box<dyn Fn(&gpui::MouseDownEvent, &mut gpui::Window, &mut gpui::App)>,
                                )
                            })
                            .collect(),
                        t,
                        cx,
                    ),
                ))
                .child(row(
                    "数值千分位逗号",
                    "为 int4 / int8 数值列启用千分位逗号",
                    self.toggle_pill(
                        "set-thousand",
                        s.thousand_sep,
                        t,
                        cx.listener(|this, _, _, cx| {
                            this.data.settings.thousand_sep = !this.data.settings.thousand_sep;
                            save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                            cx.notify();
                        }),
                    ),
                ))
                .child(row(
                    "外键快捷跳转",
                    "在外键列显示跳转图标，支持一键定位关联表",
                    self.toggle_pill(
                        "set-fk-jump",
                        s.show_fk_jump,
                        t,
                        cx.listener(|this, _, _, cx| {
                            this.data.settings.show_fk_jump = !this.data.settings.show_fk_jump;
                            save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                            cx.notify();
                        }),
                    ),
                ))
                .into_any_element(),

            SettingsCategory::Editor => div()
                .rounded(px(9.0))
                .bg(t.surface_1)
                .border_1()
                .border_color(t.border)
                .overflow_hidden()
                .child(row(
                    "显示行号",
                    "在 SQL 编辑器左侧显示行号槽",
                    self.toggle_pill(
                        "set-line-nums",
                        s.show_line_numbers,
                        t,
                        cx.listener(|this, _, window, cx| {
                            this.data.settings.show_line_numbers =
                                !this.data.settings.show_line_numbers;
                            let show = this.data.settings.show_line_numbers;
                            for sql in this.data.tabs.iter().filter_map(|tab| tab.sql_state.as_ref()) {
                                sql.editor.update(cx, |ed, cx| ed.set_line_number(show, window, cx));
                            }
                            for ts in this.data.tabs.iter().filter_map(|tab| tab.table_state.as_ref()) {
                                ts.ddl_editor
                                    .update(cx, |ed, cx| ed.set_line_number(show, window, cx));
                            }
                            this.inspector_editor_input
                                .update(cx, |ed, cx| ed.set_line_number(show, window, cx));
                            save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                            cx.notify();
                        }),
                    ),
                ))
                .child(row(
                    "关键字自动大写",
                    "格式化时自动将 SELECT / WHERE 转为大写",
                    self.toggle_pill(
                        "set-upper",
                        s.auto_uppercase,
                        t,
                        cx.listener(|this, _, _, cx| {
                            this.data.settings.auto_uppercase = !this.data.settings.auto_uppercase;
                            save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                            cx.notify();
                        }),
                    ),
                ))
                .into_any_element(),

            SettingsCategory::Safety => div()
                .rounded(px(9.0))
                .bg(t.surface_1)
                .border_1()
                .border_color(t.border)
                .overflow_hidden()
                .child(row(
                    "安全模式（暂存修改）",
                    "默认将表格增删改写入暂存区，手动提交后生效",
                    self.toggle_pill(
                        "set-safe-mode",
                        s.safe_mode,
                        t,
                        cx.listener(|this, _, _, cx| {
                            this.data.settings.safe_mode = !this.data.settings.safe_mode;
                            save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                            cx.notify();
                        }),
                    ),
                ))
                .child(row(
                    "生产环境提交二次确认",
                    "PROD 环境提交事务前强制弹出 SQL 预览确认",
                    self.toggle_pill(
                        "set-prod-confirm",
                        s.prod_confirm,
                        t,
                        cx.listener(|this, _, _, cx| {
                            this.data.settings.prod_confirm = !this.data.settings.prod_confirm;
                            save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                            cx.notify();
                        }),
                    ),
                ))
                .into_any_element(),

            SettingsCategory::Keys => div()
                .rounded(px(9.0))
                .bg(t.surface_1)
                .border_1()
                .border_color(t.border)
                .overflow_hidden()
                .children([
                    ("全局命令面板", "⌘ K"),
                    ("打开偏好设置", "⌘ ,"),
                    ("运行 SQL 查询", "⌘ ⏎"),
                    ("新建 SQL 标签页", "⌘ T"),
                    ("关闭弹窗", "Esc"),
                ].into_iter().map(|(name, k)| {
                    div()
                        .px(px(14.0))
                        .py(px(9.0))
                        .border_b_1()
                        .border_color(t.border)
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_size(t.fs_md()).child(name))
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(t.surface_2)
                                .border_1()
                                .border_color(t.border)
                                .text_size(t.fs_sm())
                                .child(k),
                        )
                }))
                .into_any_element(),

            SettingsCategory::Mcp => div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .rounded(px(9.0))
                        .bg(t.surface_1)
                        .border_1()
                        .border_color(t.border)
                        .overflow_hidden()
                        .child(row(
                            "启用本地 MCP 服务",
                            "允许外部 AI Agent（Claude Code / Cursor / Antigravity）接入",
                            self.toggle_pill(
                                "set-mcp-en",
                                s.mcp_enabled,
                                t,
                                cx.listener(|this, _, _, cx| {
                                    this.data.settings.mcp_enabled =
                                        !this.data.settings.mcp_enabled;
                                    save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                                    cx.notify();
                                }),
                            ),
                        ))
                        .child(row(
                            "Agent 只读安全锁",
                            "禁止外部 Agent 执行 INSERT / UPDATE / DELETE / DDL",
                            self.toggle_pill(
                                "set-mcp-ro",
                                s.mcp_read_only,
                                t,
                                cx.listener(|this, _, _, cx| {
                                    this.data.settings.mcp_read_only =
                                        !this.data.settings.mcp_read_only;
                                    save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                                    cx.notify();
                                }),
                            ),
                        )),
                )
                .child(
                    div()
                        .id("open-mcp-json-btn")
                        .self_end()
                        .h(px(28.0))
                        .px(px(12.0))
                        .rounded(px(6.0))
                        .bg(t.accent)
                        .text_color(gpui::white())
                        .text_size(t.fs_md())
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.data.active_modal = ActiveModal::McpConfig;
                            cx.notify();
                        }))
                        .child("查看并复制 mcp_config.json"),
                )
                .into_any_element(),

            SettingsCategory::Sync => {
                let sync = &self.data.sync_config;
                let busy = self.data.sync_busy;
                let conn_count = self.data.connections.len();

                let field_box = |label: &'static str, input_el: AnyElement| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_size(t.fs_sm())
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(t.text_2)
                                .child(label),
                        )
                        .child(
                            div()
                                .rounded(px(6.0))
                                .bg(t.panel_bg)
                                .border_1()
                                .border_color(t.border)
                                .px(px(8.0))
                                .py(px(1.0))
                                .child(input_el),
                        )
                };

                let provider_form: AnyElement = match sync.provider {
                    SyncProvider::S3 => div()
                        .rounded(px(9.0))
                        .bg(t.surface_1)
                        .border_1()
                        .border_color(t.border)
                        .p(px(14.0))
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(
                            div()
                                .text_size(t.fs_sm())
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(t.text_2)
                                .child("S3 兼容存储配置（支持 AWS S3 / Cloudflare R2 / MinIO / 阿里云 OSS / 腾讯云 COS）"),
                        )
                        .child(
                            div()
                                .grid()
                                .grid_cols(2)
                                .gap(px(10.0))
                                .child(field_box(
                                    "Endpoint 地址",
                                    Input::new(&self.sync_s3_endpoint_input)
                                        .appearance(false)
                                        .px_0()
                                        .py_0()
                                        .h(t.control_h())
                                        .text_size(t.fs_md())
                                        .into_any_element(),
                                ))
                                .child(field_box(
                                    "Region 区域",
                                    Input::new(&self.sync_s3_region_input)
                                        .appearance(false)
                                        .px_0()
                                        .py_0()
                                        .h(t.control_h())
                                        .text_size(t.fs_md())
                                        .into_any_element(),
                                ))
                                .child(field_box(
                                    "Bucket 存储桶",
                                    Input::new(&self.sync_s3_bucket_input)
                                        .appearance(false)
                                        .px_0()
                                        .py_0()
                                        .h(t.control_h())
                                        .text_size(t.fs_md())
                                        .into_any_element(),
                                ))
                                .child(field_box(
                                    "远端对象路径 (Object Key)",
                                    Input::new(&self.sync_s3_key_input)
                                        .appearance(false)
                                        .px_0()
                                        .py_0()
                                        .h(t.control_h())
                                        .text_size(t.fs_md())
                                        .into_any_element(),
                                ))
                                .child(field_box(
                                    "Access Key ID",
                                    Input::new(&self.sync_s3_ak_input)
                                        .appearance(false)
                                        .px_0()
                                        .py_0()
                                        .h(t.control_h())
                                        .text_size(t.fs_md())
                                        .into_any_element(),
                                ))
                                .child(field_box(
                                    "Secret Access Key",
                                    Input::new(&self.sync_s3_sk_input)
                                        .appearance(false)
                                        .px_0()
                                        .py_0()
                                        .h(t.control_h())
                                        .text_size(t.fs_md())
                                        .into_any_element(),
                                )),
                        )
                        .child(field_box(
                            "Session Token（可选，临时凭证使用）",
                            Input::new(&self.sync_s3_token_input)
                                .appearance(false)
                                .px_0()
                                .py_0()
                                .h(t.control_h())
                                .text_size(t.fs_md())
                                .into_any_element(),
                        ))
                        .child(
                            div()
                                .pt(px(4.0))
                                .border_t_1()
                                .border_color(t.border)
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.0))
                                        .child(
                                            div()
                                                .text_size(t.fs_md())
                                                .font_weight(gpui::FontWeight::MEDIUM)
                                                .child("Path-Style 路径风格访问"),
                                        )
                                        .child(
                                            div()
                                                .text_size(t.fs_sm())
                                                .text_color(t.text_3)
                                                .child("MinIO / R2 / 自建 S3 推荐开启；OSS 等可按需关闭"),
                                        ),
                                )
                                .child(self.toggle_pill(
                                    "set-sync-path-style",
                                    sync.s3.path_style,
                                    t,
                                    cx.listener(|this, _, _, cx| {
                                        this.data.sync_config.s3.path_style =
                                            !this.data.sync_config.s3.path_style;
                                        save_sync_config(&this.data.sync_config);
                                        cx.notify();
                                    }),
                                )),
                        )
                        .into_any_element(),

                    SyncProvider::WebDav => div()
                        .rounded(px(9.0))
                        .bg(t.surface_1)
                        .border_1()
                        .border_color(t.border)
                        .p(px(14.0))
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(
                            div()
                                .text_size(t.fs_sm())
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(t.text_2)
                                .child("WebDAV 配置（支持坚果云 / Nextcloud / ownCloud / 群晖 NAS / Alist）"),
                        )
                        .child(field_box(
                            "WebDAV 服务地址 (Endpoint URL)",
                            Input::new(&self.sync_dav_endpoint_input)
                                .appearance(false)
                                .px_0()
                                .py_0()
                                .h(t.control_h())
                                .text_size(t.fs_md())
                                .into_any_element(),
                        ))
                        .child(
                            div()
                                .grid()
                                .grid_cols(2)
                                .gap(px(10.0))
                                .child(field_box(
                                    "用户名",
                                    Input::new(&self.sync_dav_user_input)
                                        .appearance(false)
                                        .px_0()
                                        .py_0()
                                        .h(t.control_h())
                                        .text_size(t.fs_md())
                                        .into_any_element(),
                                ))
                                .child(field_box(
                                    "密码 / 应用专用密码",
                                    Input::new(&self.sync_dav_pass_input)
                                        .appearance(false)
                                        .px_0()
                                        .py_0()
                                        .h(t.control_h())
                                        .text_size(t.fs_md())
                                        .into_any_element(),
                                )),
                        )
                        .child(field_box(
                            "远端文件路径（自动创建父目录）",
                            Input::new(&self.sync_dav_path_input)
                                .appearance(false)
                                .px_0()
                                .py_0()
                                .h(t.control_h())
                                .text_size(t.fs_md())
                                .into_any_element(),
                        ))
                        .into_any_element(),
                };

                let status_text = self
                    .data
                    .sync_status_msg
                    .clone()
                    .unwrap_or_else(|| {
                        format!(
                            "当前本地共 {conn_count} 个主机连接 · 同步文件保存于远端配置快照"
                        )
                    });

                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    // Enable + Provider selector
                    .child(
                        div()
                            .rounded(px(9.0))
                            .bg(t.surface_1)
                            .border_1()
                            .border_color(t.border)
                            .overflow_hidden()
                            .child(row(
                                "启用云端配置同步",
                                "跨设备同步全部主机连接配置（含凭证/SSH 配置）与全部应用偏好设置",
                                self.toggle_pill(
                                    "set-sync-enabled",
                                    sync.enabled,
                                    t,
                                    cx.listener(|this, _, _, cx| {
                                        this.data.sync_config.enabled =
                                            !this.data.sync_config.enabled;
                                        save_sync_config(&this.data.sync_config);
                                        cx.notify();
                                    }),
                                ),
                            ))
                            .child(
                                div()
                                    .px(px(14.0))
                                    .py(px(10.0))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(12.0))
                                    .child(
                                        div()
                                            .text_size(t.fs_md())
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .child("存储后端协议"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(6.0))
                                            .children(
                                                [SyncProvider::S3, SyncProvider::WebDav]
                                                    .into_iter()
                                                    .enumerate()
                                                    .map(|(i, prov)| {
                                                        let active = sync.provider == prov;
                                                        div()
                                                            .id(("sync-prov-pill", i))
                                                            .h(px(28.0))
                                                            .px(px(12.0))
                                                            .rounded(px(6.0))
                                                            .bg(if active {
                                                                t.accent_soft
                                                            } else {
                                                                t.panel_bg
                                                            })
                                                            .border_1()
                                                            .border_color(if active {
                                                                t.accent
                                                            } else {
                                                                t.border
                                                            })
                                                            .text_size(t.fs_md())
                                                            .font_weight(if active {
                                                                gpui::FontWeight::SEMIBOLD
                                                            } else {
                                                                gpui::FontWeight::NORMAL
                                                            })
                                                            .text_color(if active {
                                                                t.accent
                                                            } else {
                                                                t.text_2
                                                            })
                                                            .flex()
                                                            .items_center()
                                                            .cursor_pointer()
                                                            .hover(|s| s.border_color(t.accent))
                                                            .on_click(cx.listener(
                                                                move |this, _, _, cx| {
                                                                    this.data.sync_config.provider =
                                                                        prov;
                                                                    save_sync_config(
                                                                        &this.data.sync_config,
                                                                    );
                                                                    cx.notify();
                                                                },
                                                            ))
                                                            .child(prov.display_name())
                                                    }),
                                            ),
                                    ),
                            ),
                    )
                    // Provider-specific form
                    .child(provider_form)
                    // Sync strategy & automation toggles
                    .child(
                        div()
                            .rounded(px(9.0))
                            .bg(t.surface_1)
                            .border_1()
                            .border_color(t.border)
                            .child(row(
                                "拉取冲突策略",
                                "从云端拉取时对主机连接列表的处理方式",
                                self.select_dropdown(
                                    "set-sync-strategy",
                                    sync.pull_strategy.display_name().into(),
                                    false,
                                    vec![
                                        (
                                            "合并主机（保留本地独有）",
                                            sync.pull_strategy == SyncPullStrategy::Merge,
                                            Box::new(cx.listener(|this, _, _, cx| {
                                                this.data.sync_config.pull_strategy =
                                                    SyncPullStrategy::Merge;
                                                this.data.open_dropdown = None;
                                                save_sync_config(&this.data.sync_config);
                                                cx.notify();
                                            })),
                                        ),
                                        (
                                            "完全覆盖本地",
                                            sync.pull_strategy == SyncPullStrategy::Overwrite,
                                            Box::new(cx.listener(|this, _, _, cx| {
                                                this.data.sync_config.pull_strategy =
                                                    SyncPullStrategy::Overwrite;
                                                this.data.open_dropdown = None;
                                                save_sync_config(&this.data.sync_config);
                                                cx.notify();
                                            })),
                                        ),
                                    ],
                                    t,
                                    cx,
                                ),
                            ))
                            .child(row(
                                "同步 SQL 片段与收藏表",
                                "除主机连接与设置外，同时同步已保存的 SQL 片段与收藏表",
                                self.toggle_pill(
                                    "set-sync-extras",
                                    sync.sync_snippets && sync.sync_favorites,
                                    t,
                                    cx.listener(|this, _, _, cx| {
                                        let next = !(this.data.sync_config.sync_snippets
                                            && this.data.sync_config.sync_favorites);
                                        this.data.sync_config.sync_snippets = next;
                                        this.data.sync_config.sync_favorites = next;
                                        save_sync_config(&this.data.sync_config);
                                        cx.notify();
                                    }),
                                ),
                            ))
                            .child(row(
                                "启动时自动拉取配置",
                                "打开 VeloDB 时自动从云端同步最新主机配置与设置",
                                self.toggle_pill(
                                    "set-sync-auto-startup",
                                    sync.auto_sync_on_startup,
                                    t,
                                    cx.listener(|this, _, _, cx| {
                                        this.data.sync_config.auto_sync_on_startup =
                                            !this.data.sync_config.auto_sync_on_startup;
                                        save_sync_config(&this.data.sync_config);
                                        cx.notify();
                                    }),
                                ),
                            ))
                            .child(row(
                                "变更时自动上传",
                                "新增/修改主机连接或调整设置后自动静默上传至云端",
                                self.toggle_pill(
                                    "set-sync-auto-push",
                                    sync.auto_push_on_change,
                                    t,
                                    cx.listener(|this, _, _, cx| {
                                        this.data.sync_config.auto_push_on_change =
                                            !this.data.sync_config.auto_push_on_change;
                                        save_sync_config(&this.data.sync_config);
                                        cx.notify();
                                    }),
                                ),
                            )),
                    )
                    // Status bar & action buttons
                    .child(
                        div()
                            .px(px(12.0))
                            .py(px(9.0))
                            .rounded(px(8.0))
                            .bg(t.surface_2)
                            .border_1()
                            .border_color(t.border)
                            .text_size(t.fs_sm())
                            .text_color(t.text_2)
                            .child(status_text),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .id("sync-save-btn")
                                            .h(px(30.0))
                                            .px(px(12.0))
                                            .rounded(px(6.0))
                                            .bg(t.surface_2)
                                            .border_1()
                                            .border_color(t.border)
                                            .text_size(t.fs_md())
                                            .text_color(t.text_1)
                                            .flex()
                                            .items_center()
                                            .cursor_pointer()
                                            .hover(|s| s.bg(t.surface_3))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.save_sync_settings_from_inputs(cx);
                                            }))
                                            .child("保存配置"),
                                    )
                                    .child(
                                        div()
                                            .id("sync-test-btn")
                                            .h(px(30.0))
                                            .px(px(12.0))
                                            .rounded(px(6.0))
                                            .bg(t.surface_2)
                                            .border_1()
                                            .border_color(t.border)
                                            .text_size(t.fs_md())
                                            .text_color(t.text_1)
                                            .flex()
                                            .items_center()
                                            .cursor_pointer()
                                            .hover(|s| s.bg(t.surface_3))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.test_sync_connection_action(cx);
                                            }))
                                            .child(if busy { "处理中…" } else { "测试连接" }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .id("sync-pull-btn")
                                            .h(px(30.0))
                                            .px(px(12.0))
                                            .rounded(px(6.0))
                                            .bg(t.surface_2)
                                            .border_1()
                                            .border_color(t.border_strong)
                                            .text_size(t.fs_md())
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(t.text_1)
                                            .flex()
                                            .items_center()
                                            .gap(px(5.0))
                                            .cursor_pointer()
                                            .hover(|s| s.bg(t.surface_3))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.pull_sync_now(false, cx);
                                            }))
                                            .child(hgi("download", 13.0, t.text_1))
                                            .child("从云端拉取"),
                                    )
                                    .child(
                                        div()
                                            .id("sync-push-btn")
                                            .h(px(30.0))
                                            .px(px(13.0))
                                            .rounded(px(6.0))
                                            .bg(t.accent)
                                            .text_color(gpui::white())
                                            .text_size(t.fs_md())
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .flex()
                                            .items_center()
                                            .gap(px(5.0))
                                            .cursor_pointer()
                                            .hover(|s| s.opacity(0.92))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.push_sync_now(false, cx);
                                            }))
                                            .child(hgi("upload", 13.0, gpui::white()))
                                            .child("上传到云端"),
                                    ),
                            ),
                    )
                    .into_any_element()
            }

            SettingsCategory::Advanced => {
                let toml_preview = s.to_toml_string();
                let toml_copy = toml_preview.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        div()
                            .text_size(t.fs_sm())
                            .text_color(t.text_3)
                            .child("实时配置预览 (~/.config/velodb/settings.toml)"),
                    )
                    .child(
                        div()
                            .p(px(12.0))
                            .rounded(px(6.0))
                            .bg(t.surface_1)
                            .border_1()
                            .border_color(t.border)
                            .max_h(px(260.0))
                            .overflow_y_scrollbar()
                            .text_size(t.fs_sm())
                            .child(toml_preview),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .id("copy-toml-btn")
                                    .h(px(28.0))
                                    .px(px(12.0))
                                    .rounded(px(6.0))
                                    .bg(t.accent)
                                    .text_color(gpui::white())
                                    .text_size(t.fs_md())
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                            toml_copy.clone(),
                                        ));
                                        this.show_toast("已复制 settings.toml", cx);
                                    }))
                                    .child("复制 settings.toml"),
                            )
                            .child(
                                div()
                                    .id("reset-settings-btn")
                                    .h(px(28.0))
                                    .px(px(12.0))
                                    .rounded(px(6.0))
                                    .bg(t.red_soft)
                                    .text_color(t.red)
                                    .text_size(t.fs_md())
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.data.settings = AppSettings::default();
                                        save_settings(&this.data.settings);
                                    this.maybe_auto_push_sync(cx);
                                        this.show_toast("已恢复默认设置", cx);
                                    }))
                                    .child("恢复默认设置"),
                            ),
                    )
                    .into_any_element()
            }
        };

        div()
            .id("settings-modal-card")
            .w(px(780.0))
            .h(px(560.0))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border_strong)
            .rounded(px(16.0))
            .shadow_lg()
            .overflow_hidden()
            .flex()
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                div()
                    .w(px(210.0))
                    .flex_shrink_0()
                    .bg(t.sidebar_bg)
                    .border_r_1()
                    .border_color(t.border)
                    .p(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .px(px(10.0))
                            .pb(px(8.0))
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_size(t.fs_sm())
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.text_3)
                            .child("偏好设置")
                            .child("⌘,"),
                    )
                    .children(nav_items.into_iter().enumerate().map(
                        |(i, (c, top_hex, bot_hex, ico, label))| {
                            let active = cat == c;
                            div()
                                .id(("snav", i))
                                .px(px(10.0))
                                .py(px(7.5))
                                .rounded(px(6.0))
                                .flex()
                                .items_center()
                                .gap(px(9.0))
                                .text_size(t.fs_md())
                                .cursor_pointer()
                                .bg(if active {
                                    t.accent_soft
                                } else {
                                    gpui::transparent_black()
                                })
                                .text_color(if active { t.accent } else { t.text_2 })
                                .font_weight(if active {
                                    gpui::FontWeight::SEMIBOLD
                                } else {
                                    gpui::FontWeight::NORMAL
                                })
                                .hover(|s| s.bg(t.surface_2))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.data.settings_cat = c;
                                    cx.notify();
                                }))
                                .child(macos_icon(top_hex, bot_hex, ico))
                                .child(label)
                        },
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .px(px(20.0))
                            .py(px(14.0))
                            .border_b_1()
                            .border_color(t.border)
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_size(t.fs_lg())
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(cat.title()),
                                    )
                                    .child(
                                        div()
                                            .text_size(t.fs_sm())
                                            .text_color(t.text_3)
                                            .child(cat.desc()),
                                    ),
                            )
                            .child(
                                div()
                                    .id("settings-close-x")
                                    .w(px(24.0))
                                    .h(px(24.0))
                                    .rounded(px(6.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.data.active_modal = ActiveModal::None;
                                        cx.notify();
                                    }))
                                    .child(hgi("cancel", 14.0, t.text_2)),
                            ),
                    )
                    .child(div().flex_1().overflow_y_scrollbar().p(px(18.0)).child(pane_body)),
            )
            .into_any_element()
    }

    fn select_dropdown(
        &self,
        id: &'static str,
        current_label: String,
        full_width: bool,
        options: Vec<(
            &'static str,
            bool,
            Box<dyn Fn(&gpui::MouseDownEvent, &mut gpui::Window, &mut gpui::App)>,
        )>,
        t: &VeloTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_open = self.data.open_dropdown == Some(id);
        div()
            .relative()
            .when(full_width, |d| d.w_full())
            .child(
                div()
                    .id(id)
                    .h(if full_width { px(32.0) } else { px(28.0) })
                    .min_w(if full_width { px(120.0) } else { px(108.0) })
                    .px(px(10.0))
                    .rounded(px(6.0))
                    .bg(if full_width { t.surface_1 } else { t.panel_bg })
                    .border_1()
                    .border_color(if is_open { t.accent } else { t.border })
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .text_size(t.fs_md())
                    .text_color(t.text_1)
                    .cursor_pointer()
                    .hover(|s| s.border_color(t.border_strong))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if this.data.open_dropdown == Some(id) {
                                this.data.open_dropdown = None;
                            } else {
                                this.data.open_dropdown = Some(id);
                            }
                            cx.notify();
                        }),
                    )
                    .child(current_label)
                    .child(hgi("arrow-down", 12.0, t.text_3)),
            )
            .when(is_open, |d| {
                d.child(
                    deferred(
                        div()
                            .id(gpui::SharedString::from(format!("{id}-menu")))
                            .absolute()
                            .top(if full_width { px(36.0) } else { px(32.0) })
                            .left_0()
                            .when(full_width, |d| d.right_0())
                            .when(!full_width, |d| d.min_w(px(128.0)))
                            .occlude()
                            .rounded(px(8.0))
                            .bg(t.panel_bg)
                            .border_1()
                            .border_color(t.border_strong)
                            .shadow_lg()
                            .p(px(4.0))
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .children(options.into_iter().enumerate().map(
                                |(idx, (label, active, on_select))| {
                                    div()
                                        .id(gpui::SharedString::from(format!("{id}-opt-{idx}")))
                                        .px(px(8.0))
                                        .py(px(6.0))
                                        .rounded(px(5.0))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap(px(8.0))
                                        .text_size(t.fs_md())
                                        .cursor_pointer()
                                        .bg(if active {
                                            t.accent_soft
                                        } else {
                                            gpui::transparent_black()
                                        })
                                        .text_color(if active { t.accent } else { t.text_1 })
                                        .font_weight(if active {
                                            gpui::FontWeight::SEMIBOLD
                                        } else {
                                            gpui::FontWeight::NORMAL
                                        })
                                        .hover(|s| s.bg(t.surface_2))
                                        .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
                                            cx.stop_propagation();
                                            on_select(ev, window, cx);
                                        })
                                        .child(label)
                                        .when(active, |d| {
                                            d.child(div().text_size(t.fs_xs()).child("✓"))
                                        })
                                },
                            )),
                    )
                    .with_priority(20),
                )
            })
            .into_any_element()
    }

    fn toggle_pill(
        &self,
        id: &'static str,
        checked: bool,
        t: &VeloTheme,
        on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .w(px(36.0))
            .h(px(20.0))
            .rounded(px(99.0))
            .bg(if checked { t.accent } else { t.surface_3 })
            .p(px(2.0))
            .flex()
            .items_center()
            .justify_end()
            .when(!checked, |d| d.justify_start())
            .cursor_pointer()
            .on_click(on_click)
            .child(div().w(px(16.0)).h(px(16.0)).rounded_full().bg(gpui::white()))
            .into_any_element()
    }
}
