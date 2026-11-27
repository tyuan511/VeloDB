use crate::adapter::postgres::format_sql;
use crate::config::save_settings;
use crate::ui::app::VeloDbApp;
use crate::ui::data_grid::{
    compact_cell_preview, grid_rows, grid_viewport, GridColumns, VisibleColumns,
};
use crate::ui::icons::hgi;
use crate::ui::sidebar::{build_sidebar_tooltip, SIDEBAR_TOOLTIP_DELAY};
use crate::ui::state::{
    ActiveModal, ColResizeState, ColResizeTarget, DrawerTab, SqlResultTab, SqlTabState,
};
use crate::ui::theme::VeloTheme;
use gpui::{
    div, prelude::*, px, AnyElement, Context, InteractiveElement, MouseButton,
    ParentElement, Styled,
};
use gpui_component::input::Editor;
use gpui_component::scroll::ScrollableElement;
use std::rc::Rc;

pub(super) fn sql_preview(sql: &str) -> String {
    const MAX_CHARS: usize = 180;
    let mut chars = sql.chars();
    let mut preview: String = chars.by_ref().take(MAX_CHARS)
        .map(|ch| if matches!(ch, '\n' | '\r') { ' ' } else { ch })
        .collect();
    if chars.next().is_some() {
        preview.push('…');
    }
    preview
}

impl SqlTabState {
    pub(super) fn column_width(&self, column: usize) -> f32 {
        self.col_widths.get(&column).copied().unwrap_or(160.0)
    }
}

impl VeloDbApp {
    fn sql_header_cells(
        &self,
        visible: VisibleColumns<'_>,
        t: &VeloTheme,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let Some(result) = self
            .active_tab()
            .and_then(|tab| tab.sql_state.as_ref())
            .and_then(|ss| ss.result.as_ref())
        else {
            return div();
        };
        div()
            .flex()
            .items_center()
            .pl(visible.leading_space)
            .children(visible.cells.iter().map(|column| {
                let ci = column.index;
                let width = column.width;
                let name = &result.columns[ci];
                let col_tip = name.clone();
                let col_tip_theme = *t;
                let is_resizing_col = matches!(
                    &self.data.col_resizing,
                    Some(ColResizeState {
                        target: ColResizeTarget::Sql {
                            col_idx
                        },
                        ..
                    }) if *col_idx == ci
                );
                let row_h = t.sql_row_h();
                div()
                    .id(("sql-th", ci))
                    .relative()
                    .w(width)
                    .h(row_h)
                    .flex_shrink_0()
                    .px(px(10.0))
                    .border_r_1()
                    .border_color(t.border)
                    .flex()
                    .items_center()
                    .text_size(t.fs_md())
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                    .tooltip(move |window, cx| {
                        build_sidebar_tooltip(col_tip.clone(), col_tip_theme, window, cx)
                    })
                    .child(div().w_full().min_w_0().truncate().child(name.clone()))
                    .child(
                        div()
                            .id(("sql-th-resize", ci))
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(0.0))
                            .w(px(7.0))
                            .cursor_col_resize()
                            .flex()
                            .justify_end()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    if ev.click_count >= 2 {
                                        this.data.col_resizing = None;
                                        if let Some(ss) =
                                            this.active_tab_mut().and_then(|t| t.sql_state.as_mut())
                                        {
                                            ss.col_widths.remove(&ci);
                                        }
                                    } else {
                                        this.data.col_resizing = Some(ColResizeState {
                                            target: ColResizeTarget::Sql { col_idx: ci },
                                            start_x: f32::from(ev.position.x),
                                            start_width: f32::from(width),
                                        });
                                    }
                                    cx.notify();
                                }),
                            )
                            .on_click(|_, _, cx| {
                                cx.stop_propagation();
                            })
                            .child(
                                div()
                                    .w(px(2.0))
                                    .h_full()
                                    .bg(if is_resizing_col {
                                        t.accent
                                    } else {
                                        gpui::transparent_black()
                                    })
                                    .hover(|s| s.bg(t.accent)),
                            ),
                    )
            }))
    }

    fn render_sql_row(
        &self,
        ss: &SqlTabState,
        ri: usize,
        row: &[Option<String>],
        visible: VisibleColumns<'_>,
        t: &VeloTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_sel_row = ss.selected_row == Some(ri);
        let row_h = t.sql_row_h();
        div()
            .h(row_h)
            .flex_shrink_0()
            .flex()
            .items_center()
            .bg(if is_sel_row {
                t.row_selected
            } else {
                gpui::transparent_black()
            })
            .border_b_1()
            .border_color(t.border)
            .hover(|s| s.bg(t.row_hover))
            .pl(visible.leading_space)
            .children(
                visible
                    .cells
                    .iter()
                    .take_while(|c| c.index < row.len())
                    .map(|column| {
                        let ci = column.index;
                        let cell = &row[ci];
                        let is_sel_cell = is_sel_row && ss.selected_col_idx == Some(ci);
                        let display = cell
                            .as_deref()
                            .map(|s| compact_cell_preview(s, 120))
                            .unwrap_or_else(|| self.data.settings.null_label.clone());
                        div()
                            .id(("sql-cell", ri * 10_000 + ci))
                            .relative()
                            .w(column.width)
                            .h(row_h)
                            .flex_shrink_0()
                            .px(px(10.0))
                            .border_r_1()
                            .border_color(t.border)
                            .flex()
                            .items_center()
                            .text_size(t.fs_md())
                            .overflow_hidden()
                            .cursor_pointer()
                            .text_color(if cell.is_none() { t.text_3 } else { t.text_1 })
                            .on_click(cx.listener(
                                move |this, ev: &gpui::ClickEvent, window, cx| {
                                    this.focus_handle.focus(window, cx);
                                    if let Some(ss) =
                                        this.active_tab_mut().and_then(|t| t.sql_state.as_mut())
                                    {
                                        ss.selected_row = Some(ri);
                                        ss.selected_col_idx = Some(ci);
                                    }
                                    if ev.click_count() >= 2 {
                                        this.data.drawer_open = true;
                                        this.data.drawer_tab = DrawerTab::Cell;
                                    }
                                    this.sync_inspector_for_active_cell(window, cx);
                                    cx.notify();
                                },
                            ))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(ss) =
                                        this.active_tab_mut().and_then(|t| t.sql_state.as_mut())
                                    {
                                        ss.selected_row = Some(ri);
                                        ss.selected_col_idx = Some(ci);
                                    }
                                    this.data.sidebar_ctx_menu.open = false;
                                    this.data.tab_ctx_menu.open = false;
                                    this.data.ctx_menu.open = true;
                                    this.data.ctx_menu.is_sql = true;
                                    this.data.ctx_menu.x = ev.position.x.into();
                                    this.data.ctx_menu.y = ev.position.y.into();
                                    this.data.ctx_menu.row_idx = ri;
                                    this.data.ctx_menu.col_idx = ci;
                                    cx.notify();
                                }),
                            )
                            .child(div().w_full().min_w_0().truncate().child(display))
                            .when(is_sel_cell, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .border_1()
                                        .border_color(t.accent),
                                )
                            })
                    }),
            )
            .into_any_element()
    }

    pub fn render_sql_workbench(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let Some(ss) = self.active_tab().and_then(|tab| tab.sql_state.as_ref()) else {
            return div().into_any_element();
        };

        let res_count = ss.result.as_ref().map(|r| r.rows.len()).unwrap_or(0);
        let is_loading = ss.is_loading;
        let timing_text = if is_loading {
            "正在执行...".to_string()
        } else {
            ss.result
                .as_ref()
                .map(|r| format!("{:.1} ms · {} 行", r.elapsed_ms, r.rows.len()))
                .unwrap_or_else(|| "未执行".into())
        };

        let res_tab_btn = |id: &'static str, label: String, target: SqlResultTab| {
            let active = ss.result_tab == target;
            div()
                .id(id)
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(4.0))
                .text_size(t.fs_sm())
                .font_weight(gpui::FontWeight::MEDIUM)
                .cursor_pointer()
                .bg(if active {
                    t.panel_bg
                } else {
                    gpui::transparent_black()
                })
                .text_color(if active { t.text_1 } else { t.text_2 })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_sql_result_tab(target, cx);
                }))
                .child(label)
        };

        let result_body: AnyElement = match ss.result_tab {
            SqlResultTab::Result => {
                if let Some(res) = &ss.result {
                    if res.columns.is_empty() {
                        div()
                            .p(px(16.0))
                            .text_size(t.fs_md())
                            .text_color(t.green)
                            .child(res.message.clone())
                            .into_any_element()
                    } else {
                        let columns = Rc::new(GridColumns::new(
                            px(0.0),
                            (0..res.columns.len()).map(|ci| (ci, px(ss.column_width(ci)))),
                        ));
                        let total_width = columns.width;
                        let row_h = t.sql_row_h();
                        let theme = *t;
                        let header = self.grid_header(
                            row_h,
                            t,
                            &columns,
                            &ss.horizontal_scroll,
                            cx,
                            Self::sql_header_cells,
                        );
                        let list = grid_rows(
                            gpui::SharedString::from(format!(
                                "sql-rows-{}",
                                self.data.active_tab_id
                            )),
                            res.rows.len(),
                            cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                                let Some(ss) =
                                    this.active_tab().and_then(|tab| tab.sql_state.as_ref())
                                else {
                                    return Vec::new();
                                };
                                let Some(res) = &ss.result else {
                                    return Vec::new();
                                };
                                let visible = columns.visible(&ss.horizontal_scroll, window);
                                range
                                    .map(|ri| {
                                        this.render_sql_row(
                                            ss,
                                            ri,
                                            &res.rows[ri],
                                            visible,
                                            &theme,
                                            cx,
                                        )
                                    })
                                    .collect()
                            }),
                        )
                        .flex_1()
                        .min_h_0()
                        .track_scroll(&ss.vertical_scroll);

                        grid_viewport(
                            &self.data.active_tab_id,
                            total_width,
                            row_h + px(1.0),
                            header,
                            list,
                            &ss.vertical_scroll,
                            &ss.horizontal_scroll,
                        )
                    }
                } else {
                    div()
                        .p(px(20.0))
                        .text_size(t.fs_md())
                        .text_color(t.text_3)
                        .child("点击「运行 (⌘⏎)」执行查询")
                        .into_any_element()
                }
            }
            SqlResultTab::Explain => {
                if let Some(exp) = &ss.explain {
                    div()
                        .flex_1()
                        .overflow_y_scrollbar()
                        .p(px(12.0))
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            div()
                                .px(px(10.0))
                                .py(px(6.0))
                                .rounded(px(6.0))
                                .bg(t.panel_bg)
                                .border_1()
                                .border_color(t.border)
                                .flex()
                                .items_center()
                                .justify_between()
                                .text_size(t.fs_sm())
                                .text_color(t.text_2)
                                .child(format!(
                                    "总耗时：{:.2} ms · 规划：{:.2} ms · 缓存命中：{}",
                                    exp.total_time_ms, exp.planning_time_ms, exp.shared_hit_blocks
                                ))
                                .when(exp.warnings > 0, |d| {
                                    d.child(
                                        div()
                                            .px(px(6.0))
                                            .py(px(2.0))
                                            .rounded(px(4.0))
                                            .bg(t.amber_soft)
                                            .text_color(t.amber)
                                            .text_size(t.fs_xs())
                                            .child(format!("{} 处全表扫描", exp.warnings)),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .rounded(px(9.0))
                                .bg(t.panel_bg)
                                .border_1()
                                .border_color(t.border)
                                .overflow_hidden()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .flex()
                                        .px(px(10.0))
                                        .py(px(6.0))
                                        .bg(t.surface_1)
                                        .border_b_1()
                                        .border_color(t.border)
                                        .text_size(t.fs_xs())
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(t.text_3)
                                        .child(div().flex_1().child("执行算子"))
                                        .child(div().w(px(96.0)).child("实际行数"))
                                        .child(div().w(px(120.0)).child("耗时")),
                                )
                                .children(exp.nodes.iter().map(|n| {
                                    div()
                                        .flex()
                                        .items_center()
                                        .px(px(10.0))
                                        .py(px(7.0))
                                        .border_b_1()
                                        .border_color(t.border)
                                        .bg(if n.is_warn {
                                            t.amber_soft
                                        } else {
                                            gpui::transparent_black()
                                        })
                                        .text_size(t.fs_md())
                                        .child(
                                            div()
                                                .flex_1()
                                                .pl(px((n.depth * 16) as f32))
                                                .flex()
                                                .items_center()
                                                .gap(px(6.0))
                                                .child(
                                                    div()
                                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                                        .text_color(if n.is_warn {
                                                            t.amber
                                                        } else {
                                                            t.text_1
                                                        })
                                                        .child(format!(
                                                            "{}{}",
                                                            if n.depth > 0 { "└ " } else { "" },
                                                            n.operation
                                                        )),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(t.fs_sm())
                                                        .text_color(t.text_3)
                                                        .child(n.detail.clone()),
                                                ),
                                        )
                                        .child(div().w(px(96.0)).child(n.actual_rows.clone()))
                                        .child(div().w(px(120.0)).child(n.time_ms.clone()))
                                })),
                        )
                        .into_any_element()
                } else {
                    div()
                        .p(px(20.0))
                        .text_size(t.fs_md())
                        .text_color(t.text_3)
                        .child(if is_loading { "正在生成执行计划…" } else { "点击上方 EXPLAIN 查看当前 SQL 的执行计划" })
                        .into_any_element()
                }
            }
            SqlResultTab::Messages => div()
                .flex_1()
                .overflow_y_scrollbar()
                .p(px(12.0))
                .child(
                    div()
                        .p(px(12.0))
                        .rounded(px(6.0))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .text_size(t.fs_sm())
                        .child(ss.messages.join("\n")),
                )
                .into_any_element(),
            SqlResultTab::History => div()
                .flex_1()
                .overflow_y_scrollbar()
                .p(px(12.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .children(self.data.history.iter().enumerate().map(|(i, h)| {
                    let sql = h.sql.clone();
                    let editor = ss.editor.clone();
                    div()
                        .id(("hist-item", i))
                        .p(px(10.0))
                        .rounded(px(9.0))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .cursor_pointer()
                        .hover(|s| s.border_color(t.accent))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            editor.update(cx, |s, cx| s.set_value(sql.to_string(), window, cx));
                            this.show_toast("已载入历史 SQL", cx);
                        }))
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(t.fs_md())
                                        .flex_1().min_w_0().truncate()
                                        .child(sql_preview(&h.sql)),
                                )
                                .child(
                                    div()
                                        .px(px(5.0))
                                        .py(px(1.0))
                                        .rounded(px(4.0))
                                        .bg(t.surface_2)
                                        .text_size(t.fs_xs())
                                        .text_color(t.text_3)
                                        .child(h.elapsed_ms.clone()),
                                ),
                        )
                        .child(
                            div()
                                .text_size(t.fs_sm())
                                .text_color(t.text_3)
                                .child(format!("Executed at {}", h.time)),
                        )
                }))
                .into_any_element(),
        };

        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
            .relative()
            // SQL Toolbar
            .child(
                div()
                    .px(px(12.0))
                    .py(px(7.0))
                    .border_b_1()
                    .border_color(t.border)
                    .bg(t.surface_1)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .id("run-sql-btn")
                                    .h(px(28.0))
                                    .px(px(10.0))
                                    .rounded(px(6.0))
                                    .bg(t.accent)
                                    .text_color(gpui::white())
                                    .text_size(t.fs_md())
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .flex()
                                    .items_center()
                                    .gap(px(5.0))
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.run_active_sql(cx);
                                    }))
                                    .child(hgi("play", 13.0, gpui::white()))
                                    .child(if is_loading {
                                        "执行中..."
                                    } else {
                                        "运行 (⌘⏎)"
                                    }),
                            )
                            .child(
                                div()
                                    .id("explain-sql-btn")
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
                                        this.explain_active_sql(cx);
                                    }))
                                    .child(hgi("flash", 13.0, t.text_2))
                                    .child("EXPLAIN"),
                            )
                            .child(
                                div()
                                    .id("format-sql-btn")
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
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        let Some(editor) = this.active_sql_editor() else { return };
                                        let raw = editor.read(cx).text().to_string();
                                        let formatted = format_sql(
                                            &raw,
                                            this.data.settings.auto_uppercase,
                                            this.data.settings.tab_size,
                                        );
                                        editor.update(cx, |s, cx| s.set_value(formatted, window, cx));
                                        this.show_toast("SQL 已格式化", cx);
                                    }))
                                    .child(hgi("magic", 13.0, t.text_2))
                                    .child("格式化"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .id("snippets-btn")
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
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.data.active_modal = ActiveModal::SnippetsManager;
                                        cx.notify();
                                    }))
                                    .child(hgi("code", 13.0, t.text_2))
                                    .child("代码片段")
                                    .child(hgi("arrow-down", 11.0, t.text_3)),
                            )
                            .child(
                                div()
                                    .id("save-snippet-btn")
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
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.data.active_modal = ActiveModal::SnippetsManager;
                                        cx.notify();
                                    }))
                                    .child(hgi("plus", 13.0, t.text_2))
                                    .child("保存片段"),
                            ),
                    ),
            )
            // SQL Editor Pane (GPUI Code Editor + tree-sitter-sql + SQL LSP)
            .child({
                let editor_h = self.data.settings.sql_editor_height.clamp(120.0, 560.0);
                let split_resizing = self.data.sql_split_resizing.is_some();
                div()
                    .h(px(editor_h))
                    .w_full()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .relative()
                    .border_b_1()
                    .border_color(t.border)
                    .bg(t.panel_bg)
                    .child(
                        Editor::new(&ss.editor)
                            .h(px(editor_h))
                            .appearance(false)
                            .bordered(false)
                            .text_size(t.fs_md()),
                    )
                    // Bottom-edge draggable split handle (double-click to reset height)
                    .child(
                        div()
                            .id("sql-split-handle")
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom(px(-3.0))
                            .h(px(6.0))
                            .cursor_row_resize()
                            .flex()
                            .items_center()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    if ev.click_count >= 2 {
                                        this.data.sql_split_resizing = None;
                                        this.data.settings.sql_editor_height = 260.0;
                                        save_settings(&this.data.settings);
                                    } else {
                                        this.data.sql_split_resizing =
                                            Some((f32::from(ev.position.y), editor_h));
                                    }
                                    cx.notify();
                                }),
                            )
                            .child(
                                div()
                                    .h(px(2.0))
                                    .w_full()
                                    .bg(if split_resizing {
                                        t.accent
                                    } else {
                                        gpui::transparent_black()
                                    })
                                    .hover(|s| s.bg(t.accent)),
                            ),
                    )
            })
            // Results Pane
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .bg(t.surface_1)
                    .child(
                        div()
                            .px(px(12.0))
                            .py(px(6.0))
                            .border_b_1()
                            .border_color(t.border)
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .p(px(2.0))
                                    .rounded(px(6.0))
                                    .bg(t.surface_2)
                                    .border_1()
                                    .border_color(t.border)
                                    .child(res_tab_btn(
                                        "sqlres-res",
                                        format!("查询结果 ({res_count})"),
                                        SqlResultTab::Result,
                                    ))
                                    .child(res_tab_btn(
                                        "sqlres-exp",
                                        "执行计划".into(),
                                        SqlResultTab::Explain,
                                    ))
                                    .child(res_tab_btn(
                                        "sqlres-msg",
                                        "消息日志".into(),
                                        SqlResultTab::Messages,
                                    ))
                                    .child(res_tab_btn(
                                        "sqlres-hist",
                                        "查询历史".into(),
                                        SqlResultTab::History,
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(t.fs_sm())
                                    .text_color(t.green)
                                    .child(timing_text),
                            ),
                    )
                    .child(result_body),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::sql_preview;

    #[test]
    fn sql_card_preview_preserves_utf8_boundaries_and_only_marks_a_truncated_query() {
        assert_eq!(sql_preview("SELECT\n'值'"), "SELECT '值'");
        let full = "字".repeat(180);
        assert_eq!(sql_preview(&full), full);
        assert_eq!(sql_preview(&format!("{full}😀")), format!("{full}…"));
    }
}
