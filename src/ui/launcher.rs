use crate::adapter::ConnectionConfig;
use crate::config::save_connections;
use crate::ui::app::VeloDbApp;
use crate::ui::icons::{db_driver_icon, hgi};
use crate::ui::sidebar::{build_sidebar_tooltip, SIDEBAR_TOOLTIP_DELAY};
use crate::ui::state::ActiveModal;
use crate::ui::theme::{parse_hex, with_alpha, VeloTheme};
use gpui::{
    div, img, prelude::*, px, AnyElement, Context, Image, ImageFormat, InteractiveElement,
    ParentElement, Styled,
};
use gpui_component::input::Input;
use gpui_component::scroll::ScrollableElement;
use std::sync::{Arc, OnceLock};

fn app_icon() -> Arc<Image> {
    static ICON: OnceLock<Arc<Image>> = OnceLock::new();
    ICON.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../assets/app_icon_256.png").to_vec(),
        ))
    })
    .clone()
}

impl VeloDbApp {
    pub fn render_launcher(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let has_conns = !self.data.connections.is_empty();
        let title = if has_conns { "VeloDB" } else { "连接到数据库" };
        let sub = if has_conns {
            "选择一个已保存的 PostgreSQL / MySQL 连接，或新建连接开始工作。"
        } else {
            "尚未添加任何数据库连接。新建连接或直接粘贴 PostgreSQL / MySQL URI 即可开始浏览与查询。"
        };

        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .p(px(32.0))
            .overflow_y_scrollbar()
            .child(
                div()
                    .w(px(520.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(20.0))
                    // App icon
                    .child(
                        div()
                            .w(px(64.0))
                            .h(px(64.0))
                            .rounded(px(15.0))
                            .overflow_hidden()
                            .shadow_lg()
                            .child(img(app_icon()).w(px(64.0)).h(px(64.0))),
                    )
                    .child(
                        div()
                            .text_size(t.fs_2xl())
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.text_1)
                            .child(title),
                    )
                    .child(
                        div()
                            .mt(px(-10.0))
                            .text_size(t.fs_lg())
                            .text_color(t.text_2)
                            .text_center()
                            .max_w(px(380.0))
                            .child(sub),
                    )
                    // Quick URI Box
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .p(px(6.0))
                            .pl(px(12.0))
                            .rounded(px(9.0))
                            .bg(t.panel_bg)
                            .border_1()
                            .border_color(t.border_strong)
                            .shadow_sm()
                            .child(hgi("link-up", 14.0, t.text_3))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h(t.control_h())
                                    .flex()
                                    .items_center()
                                    .child(
                                        Input::new(&self.quick_uri_input)
                                            .appearance(false)
                                            .px_0()
                                            .py_0()
                                            .h_full()
                                            .text_size(t.fs_md()),
                                    ),
                            )
                            .child(
                                div()
                                    .id("quick-uri-connect-btn")
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
                                    .hover(|s| s.opacity(0.9))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let uri = this.quick_uri_input.read(cx).value().to_string();
                                        let target = uri.trim();
                                        if target.is_empty() {
                                            this.show_toast(
                                                "请输入 PostgreSQL 或 MySQL 连接 URI",
                                                cx,
                                            );
                                            return;
                                        }
                                        match ConnectionConfig::from_uri(target) {
                                            Ok(cfg) => this.connect_with_config(cfg, cx),
                                            Err(e) => this.show_toast(format!("{e}"), cx),
                                        }
                                    }))
                                    .child("直连"),
                            ),
                    )
                    // Primary CTA Row
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .id("launcher-new-conn-btn")
                                    .flex_1()
                                    .h(px(36.0))
                                    .rounded(px(9.0))
                                    .bg(t.accent)
                                    .text_color(gpui::white())
                                    .text_size(t.fs_lg())
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap(px(6.0))
                                    .cursor_pointer()
                                    .hover(|s| s.opacity(0.92))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_new_connection_modal(window, cx);
                                    }))
                                    .child(hgi("plus", 15.0, gpui::white()))
                                    .child("新建数据库连接"),
                            )
                            .child(
                                div()
                                    .id("launcher-import-conn-btn")
                                    .h(px(36.0))
                                    .px(px(14.0))
                                    .rounded(px(9.0))
                                    .bg(t.surface_2)
                                    .border_1()
                                    .border_color(t.border)
                                    .text_color(t.text_1)
                                    .text_size(t.fs_md())
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.surface_3))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_toast("已扫描 ~/.pgpass 与本地连接配置", cx);
                                    }))
                                    .child(hgi("upload", 14.0, t.text_1))
                                    .child("导入连接"),
                            ),
                    )
                    // Saved Connections List OR Zero-Connection Empty State
                    .when(has_conns, |d| {
                        let cards: Vec<AnyElement> = self
                            .data
                            .connections
                            .iter()
                            .enumerate()
                            .map(|(idx, conn)| {
                                let c_color = parse_hex(&conn.color).unwrap_or(t.accent);
                                let connect_id = conn.id.clone();
                                let edit_id = conn.id.clone();
                                let conn_id = conn.id.clone();
                                let edit_tip_theme = *t;
                                let del_tip_theme = *t;
                                div()
                                    .id(("conn-card", idx))
                                    .flex()
                                    .items_center()
                                    .gap(px(12.0))
                                    .p(px(12.0))
                                    .rounded(px(9.0))
                                    .bg(t.panel_bg)
                                    .border_1()
                                    .border_color(t.border)
                                    .cursor_pointer()
                                    .hover(|s| s.border_color(t.accent).bg(t.surface_1))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(cfg) = this.data.connections.iter().find(|c| c.id == connect_id).cloned() {
                                            this.connect_with_config(cfg, cx);
                                        }
                                    }))
                                    .child(
                                        div()
                                            .w(px(34.0))
                                            .h(px(34.0))
                                            .rounded(px(9.0))
                                            .bg(c_color)
                                            .shadow_sm()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(db_driver_icon(conn.driver, 20.0, true)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap(px(2.0))
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(8.0))
                                                    .child(
                                                        div()
                                                            .text_size(t.fs_lg())
                                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                                            .text_color(t.text_1)
                                                            .child(conn.name.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .px(px(5.0))
                                                            .py(px(1.0))
                                                            .rounded(px(4.0))
                                                            .bg(with_alpha(c_color, 0.13))
                                                            .text_color(c_color)
                                                            .text_size(t.fs_xs())
                                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                                            .child(conn.label.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .px(px(5.0))
                                                            .py(px(1.0))
                                                            .rounded(px(4.0))
                                                            .bg(t.surface_2)
                                                            .text_color(t.text_3)
                                                            .text_size(t.fs_xs())
                                                            .child(conn.driver.display_name()),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_size(t.fs_sm())
                                                    .text_color(t.text_3)
                                                    .child(conn.display_host()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(4.0))
                                            .child(
                                                div()
                                                    .id(("edit-conn", idx))
                                                    .w(px(26.0))
                                                    .h(px(26.0))
                                                    .rounded(px(6.0))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .cursor_pointer()
                                                    .text_color(t.text_3)
                                                    .hover(|s| s.bg(t.surface_2).text_color(t.text_1))
                                                    .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                                                    .tooltip(move |window, cx| {
                                                        build_sidebar_tooltip(
                                                            "编辑连接".to_string(),
                                                            edit_tip_theme,
                                                            window,
                                                            cx,
                                                        )
                                                    })
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            cx.stop_propagation();
                                                            if let Some(cfg) = this.data.connections.iter().find(|c| c.id == edit_id).cloned() {
                                                                this.open_edit_connection_modal(cfg, false, window, cx);
                                                            }
                                                        },
                                                    ))
                                                    .child(hgi("edit", 13.0, t.text_2)),
                                            )
                                            .child(
                                                div()
                                                    .id(("del-conn", idx))
                                                    .w(px(26.0))
                                                    .h(px(26.0))
                                                    .rounded(px(6.0))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .cursor_pointer()
                                                    .text_color(t.text_3)
                                                    .hover(|s| s.bg(t.red_soft).text_color(t.red))
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
                                                        this.data
                                                            .connections
                                                            .retain(|c| c.id != conn_id);
                                                        save_connections(&this.data.connections);
                                                        this.maybe_auto_push_sync(cx);
                                                        this.show_toast("已删除连接", cx);
                                                    }))
                                                    .child(hgi("trash", 13.0, t.text_3)),
                                            ),
                                    )
                                    .child(hgi("arrow-right", 14.0, t.text_3))
                                    .into_any_element()
                            })
                            .collect();

                        d.child(
                            div()
                                .w_full()
                                .flex()
                                .flex_col()
                                .gap(px(8.0))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .px(px(2.0))
                                        .child(
                                            div()
                                                .text_size(t.fs_sm())
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(t.text_3)
                                                .child("已保存的连接"),
                                        ),
                                )
                                .children(cards),
                        )
                    })
                    .when(!has_conns, |d| {
                        d.child(
                            div()
                                .w_full()
                                .grid()
                                .grid_cols(2)
                                .gap(px(10.0))
                                .child(
                                    div()
                                        .id("empty-mcp-card")
                                        .p(px(12.0))
                                        .rounded(px(9.0))
                                        .bg(t.panel_bg)
                                        .border_1()
                                        .border_color(t.border)
                                        .cursor_pointer()
                                        .hover(|s| s.border_color(t.border_strong).bg(t.surface_1))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.data.settings_cat =
                                                crate::ui::state::SettingsCategory::Mcp;
                                            this.data.active_modal = ActiveModal::Settings;
                                            cx.notify();
                                        }))
                                        .flex()
                                        .flex_col()
                                        .gap(px(6.0))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(6.0))
                                                .text_size(t.fs_md())
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(t.text_1)
                                                .child(hgi("plug", 14.0, t.cyan))
                                                .child("本地 MCP 服务"),
                                        )
                                        .child(
                                            div()
                                                .text_size(t.fs_sm())
                                                .text_color(t.text_3)
                                                .child("配置本地只读 MCP 供外部 Agent 调用"),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("empty-settings-card")
                                        .p(px(12.0))
                                        .rounded(px(9.0))
                                        .bg(t.panel_bg)
                                        .border_1()
                                        .border_color(t.border)
                                        .cursor_pointer()
                                        .hover(|s| s.border_color(t.border_strong).bg(t.surface_1))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.data.settings_cat =
                                                crate::ui::state::SettingsCategory::General;
                                            this.data.active_modal = ActiveModal::Settings;
                                            cx.notify();
                                        }))
                                        .flex()
                                        .flex_col()
                                        .gap(px(6.0))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(6.0))
                                                .text_size(t.fs_md())
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(t.text_1)
                                                .child(hgi("settings", 14.0, t.text_2))
                                                .child("偏好设置"),
                                        )
                                        .child(
                                            div()
                                                .text_size(t.fs_sm())
                                                .text_color(t.text_3)
                                                .child("自定义字体、毛玻璃质感与快捷键"),
                                        ),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }
}
