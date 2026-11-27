use crate::ui::app::VeloDbApp;
use crate::ui::icons::hgi;
use crate::ui::theme::VeloTheme;
use gpui::{
    div, prelude::*, px, AnyElement, Context, InteractiveElement, ParentElement, Styled,
};
use gpui_component::scroll::ScrollableElement;

impl VeloDbApp {
    pub fn render_monitor_view(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let m = self.data.monitor_metrics.as_ref();

        let card = |label: &'static str, val: String, sub: String, color: gpui::Hsla| {
            div()
                .p(px(14.0))
                .rounded(px(9.0))
                .bg(t.panel_bg)
                .border_1()
                .border_color(t.border)
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(t.fs_sm())
                        .text_color(t.text_3)
                        .child(label),
                )
                .child(
                    div()
                        .text_size(t.fs_2xl())
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(color)
                        .child(val),
                )
                .child(div().text_size(t.fs_sm()).text_color(t.text_2).child(sub))
        };

        div()
            .flex_1()
            .overflow_y_scrollbar()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(
                div()
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
                                    .text_size(t.fs_xl())
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("PostgreSQL 实时监控"),
                            )
                            .child(
                                div()
                                    .text_size(t.fs_md())
                                    .text_color(t.text_3)
                                    .child("pg_stat_activity 与 pg_stat_database 实时采样"),
                            ),
                    )
                    .child(
                        div()
                            .id("refresh-monitor-btn")
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
                                this.refresh_monitor(cx);
                                this.show_toast("已刷新实时指标", cx);
                            }))
                            .child(hgi("refresh", 13.0, t.text_2))
                            .child("刷新"),
                    ),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(4)
                    .gap(px(12.0))
                    .child(card(
                        "活跃连接数",
                        m.map(|x| format!("{} / {}", x.active_connections, x.max_connections))
                            .unwrap_or_else(|| "-".into()),
                        m.map(|x| {
                            format!(
                                "{} 活跃 · {} 空闲 · {} 等待",
                                x.active_count, x.idle_count, x.waiting_count
                            )
                        })
                        .unwrap_or_default(),
                        t.accent,
                    ))
                    .child(card(
                        "缓存命中率",
                        m.map(|x| format!("{:.2}%", x.cache_hit_ratio))
                            .unwrap_or_else(|| "-".into()),
                        "共享缓冲区健康".into(),
                        t.green,
                    ))
                    .child(card(
                        "数据库体积",
                        m.map(|x| x.db_size.clone()).unwrap_or_else(|| "-".into()),
                        "pg_database_size".into(),
                        t.text_1,
                    ))
                    .child(card(
                        "累计事务数",
                        m.map(|x| format!("{}", x.tps)).unwrap_or_else(|| "-".into()),
                        m.map(|x| format!("平均延迟 {:.1}ms", x.avg_latency_ms))
                            .unwrap_or_default(),
                        t.purple,
                    )),
            )
            .child(
                div()
                    .text_size(t.fs_lg())
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("活跃会话 (pg_stat_activity)"),
            )
            .child(
                div()
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(9.0))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .bg(t.titlebar_bg)
                            .border_b_1()
                            .border_color(t.border_strong)
                            .px(px(10.0))
                            .py(px(7.0))
                            .text_size(t.fs_md())
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(div().w(px(76.0)).child("PID"))
                            .child(div().w(px(110.0)).child("用户"))
                            .child(div().w(px(110.0)).child("状态"))
                            .child(div().w(px(80.0)).child("耗时"))
                            .child(div().flex_1().child("查询语句"))
                            .child(div().w(px(84.0)).child("操作")),
                    )
                    .when_some(m, |d, metrics| {
                        d.children(metrics.sessions.iter().enumerate().map(|(i, s)| {
                            let pid = s.pid;
                            let state_color = match s.state.as_str() {
                                "active" => t.green,
                                "idle" => t.amber,
                                _ => t.red,
                            };
                            div()
                                .flex()
                                .items_center()
                                .px(px(10.0))
                                .py(px(7.0))
                                .border_b_1()
                                .border_color(t.border)
                                .text_size(t.fs_md())
                                .child(div().w(px(76.0)).child(format!("{pid}")))
                                .child(div().w(px(110.0)).child(s.username.clone()))
                                .child(
                                    div()
                                        .w(px(110.0))
                                        .text_color(state_color)
                                        .child(format!("● {}", s.state)),
                                )
                                .child(div().w(px(80.0)).child(s.duration.clone()))
                                .child(
                                    div()
                                        .flex_1()
                                        .overflow_hidden()
                                        .text_color(t.text_2)
                                        .child(s.query.clone()),
                                )
                                .child(
                                    div().w(px(84.0)).child(
                                        div()
                                            .id(("cancel-pid", i))
                                            .h(px(22.0))
                                            .px(px(8.0))
                                            .rounded(px(5.0))
                                            .bg(t.red_soft)
                                            .text_color(t.red)
                                            .text_size(t.fs_sm())
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.cancel_session(pid, false, cx);
                                            }))
                                            .child("终止"),
                                    ),
                                )
                        }))
                    }),
            )
            .into_any_element()
    }
}
