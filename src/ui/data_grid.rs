use crate::adapter::{StagedChange, StagedChangeKind};
use crate::ui::app::VeloDbApp;
use crate::ui::icons::hgi;
use crate::ui::sidebar::{build_sidebar_tooltip, SIDEBAR_TOOLTIP_DELAY};
use crate::ui::state::{
    ActiveModal, ColResizeState, ColResizeTarget, DrawerTab, TableSubview, TableTabState,
};
use crate::ui::theme::{with_alpha, VeloTheme};
use gpui::{
    div, prelude::*, px, uniform_list, AnyElement, Context, InteractiveElement, MouseButton,
    ParentElement, Pixels, ScrollHandle, Styled, UniformListScrollHandle,
};
use gpui_component::input::Input;
use gpui_component::scroll::Scrollbar;
use gpui_component::InteractiveElementExt;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

pub(super) const ROW_NUMBER_WIDTH: f32 = 44.0;

struct TableRowLayout {
    height: Pixels,
    font_size: Pixels,
    updated_columns: HashMap<usize, HashSet<String>>,
}

pub(super) struct GridColumn {
    pub index: usize,
    pub width: Pixels,
    left: Pixels,
}

pub(super) struct GridColumns {
    gutter: Pixels,
    columns: Vec<GridColumn>,
    pub width: Pixels,
}

#[derive(Clone, Copy)]
pub(super) struct VisibleColumns<'a> {
    pub cells: &'a [GridColumn],
    pub leading_space: Pixels,
}

impl GridColumns {
    pub fn new(gutter: Pixels, widths: impl Iterator<Item = (usize, Pixels)>) -> Self {
        let mut left = gutter;
        let columns = widths
            .map(|(index, width)| {
                let column = GridColumn { index, left, width };
                left += width;
                column
            })
            .collect();
        Self {
            gutter,
            columns,
            width: left,
        }
    }

    fn all(&self) -> VisibleColumns<'_> {
        VisibleColumns {
            cells: &self.columns,
            leading_space: px(0.0),
        }
    }

    pub fn visible(&self, handle: &ScrollHandle, window: &gpui::Window) -> VisibleColumns<'_> {
        let bounds = handle.bounds();
        if self.width <= bounds.size.width {
            return self.all();
        }
        // A retained scroll layer can request a larger paint region than the viewport.
        let clip = window.content_mask().bounds;
        let origin = bounds.left() + handle.offset().x;
        self.visible_at(clip.left() - origin, clip.size.width)
    }

    fn visible_at(&self, left: Pixels, width: Pixels) -> VisibleColumns<'_> {
        // Keep one neighbor at either edge, including partially scrolled columns.
        let start = self
            .columns
            .partition_point(|c| c.left + c.width <= left)
            .saturating_sub(1);
        let end =
            (self.columns.partition_point(|c| c.left < left + width) + 1).min(self.columns.len());
        let cells = &self.columns[start..end];
        VisibleColumns {
            leading_space: cells.first().map_or(px(0.), |c| c.left - self.gutter),
            cells,
        }
    }
}

pub(super) fn grid_rows(
    id: gpui::SharedString,
    count: usize,
    render: impl Fn(std::ops::Range<usize>, &mut gpui::Window, &mut gpui::App) -> Vec<AnyElement>
        + 'static,
) -> gpui::UniformList {
    let mut rows = uniform_list(id, count, render);
    // Otherwise a purely horizontal wheel event also scrolls this vertical list.
    rows.style().restrict_scroll_to_axis = Some(true);
    rows
}

fn is_json_type(data_type: &str) -> bool {
    let dt = data_type.trim();
    dt.eq_ignore_ascii_case("json") || dt.eq_ignore_ascii_case("jsonb")
}

fn default_column_width(col: &crate::adapter::ColumnDef) -> f32 {
    if is_json_type(&col.data_type) {
        230.0
    } else if col.name == "email" {
        200.0
    } else {
        155.0
    }
}

pub(super) fn column_width(ts: &TableTabState, col: &crate::adapter::ColumnDef) -> f32 {
    ts.col_widths
        .get(&col.name)
        .copied()
        .unwrap_or_else(|| default_column_width(col))
}

pub(crate) fn compact_cell_preview(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(max_chars.min(s.len()) + 3);
    let mut count = 0usize;
    let mut prev_space = false;
    let mut truncated = false;

    for ch in s.trim().chars() {
        if ch.is_whitespace() {
            if !prev_space {
                if count >= max_chars {
                    truncated = true;
                    break;
                }
                out.push(' ');
                count += 1;
                prev_space = true;
            }
        } else {
            if count >= max_chars {
                truncated = true;
                break;
            }
            out.push(ch);
            count += 1;
            prev_space = false;
        }
    }

    if truncated {
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('…');
    }
    out
}

pub(super) fn reveal_grid_cell(
    vertical: &UniformListScrollHandle,
    horizontal: &ScrollHandle,
    row: usize,
    left: f32,
    width: f32,
) {
    vertical.scroll_to_item(row, gpui::ScrollStrategy::Nearest);
    let viewport = f32::from(horizontal.bounds().size.width);
    if viewport <= 0.0 {
        return;
    }
    let offset = horizontal.offset();
    let visible_left = -f32::from(offset.x);
    let target = visible_left.clamp((left + width - viewport).min(left), left);
    if target != visible_left {
        horizontal.set_offset(gpui::point(px(-target.max(0.0)), offset.y));
    }
}

// Keep header and rows in one horizontal viewport. Only the list owns vertical scrolling.
pub(super) fn grid_viewport(
    tab_id: &str,
    width: Pixels,
    header_height: Pixels,
    header: gpui::Div,
    body: impl IntoElement,
    vertical: &UniformListScrollHandle,
    horizontal: &ScrollHandle,
) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .relative()
        .overflow_hidden()
        .child(
            div()
                .id(gpui::SharedString::from(format!("grid-scroll-{tab_id}")))
                .size_full()
                .overflow_x_scroll()
                .lock_scroll_axis()
                .track_scroll(horizontal)
                .child(
                    div()
                        .w(width)
                        .min_w_full()
                        .h_full()
                        .flex()
                        .flex_col()
                        .child(header)
                        .child(body),
                ),
        )
        .child(
            div().absolute().inset_0().top(header_height).child(
                Scrollbar::vertical(vertical)
                    .id(gpui::SharedString::from(format!("grid-vbar-{tab_id}")))
                    .viewport_from_layout(),
            ),
        )
        .child(
            div().absolute().inset_0().child(
                Scrollbar::horizontal(horizontal)
                    .id(gpui::SharedString::from(format!("grid-hbar-{tab_id}")))
                    .viewport_from_layout(),
            ),
        )
        .into_any_element()
}

impl VeloDbApp {
    pub(super) fn grid_header(
        &self,
        height: Pixels,
        theme: &VeloTheme,
        columns: &Rc<GridColumns>,
        horizontal: &ScrollHandle,
        cx: &Context<Self>,
        render: fn(&Self, VisibleColumns<'_>, &VeloTheme, &Context<Self>) -> gpui::Div,
    ) -> gpui::Div {
        let header = div()
            .h(height + px(1.0))
            .flex_shrink_0()
            .bg(theme.titlebar_bg)
            .border_b_1()
            .border_color(theme.border_strong);
        if columns.width <= horizontal.bounds().size.width {
            return header.child(render(self, columns.all(), theme, cx).size_full());
        }
        // A wide header needs this frame's clamped offset and paint region. Both
        // are available after the horizontal viewport enters prepaint.
        let columns = Rc::clone(columns);
        let horizontal = horizontal.clone();
        let theme = *theme;
        let row = cx.processor(move |this, (), window, cx| {
            render(this, columns.visible(&horizontal, window), &theme, cx)
        });
        header.child(
            gpui::canvas(
                move |bounds, window, cx| {
                    let mut row = row((), window, cx).size_full().into_any_element();
                    row.prepaint_as_root(bounds.origin, bounds.size.into(), window, cx);
                    row
                },
                |_, mut row, window, cx| row.paint(window, cx),
            )
            .size_full(),
        )
    }

    pub fn render_table_view(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let Some(ts) = self.active_tab().and_then(|tab| tab.table_state.as_ref()) else {
            return div().into_any_element();
        };
        let is_data = ts.subview == TableSubview::Data;

        let subtoolbar = div()
            .px(px(12.0))
            .py(px(7.0))
            .border_b_1()
            .border_color(t.border)
            .bg(t.surface_1)
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .p(px(2.0))
                    .rounded(px(6.0))
                    .bg(t.surface_2)
                    .border_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .id("subview-data")
                            .px(px(10.0))
                            .py(px(4.0))
                            .rounded(px(4.0))
                            .text_size(t.fs_sm())
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .cursor_pointer()
                            .bg(if is_data {
                                t.panel_bg
                            } else {
                                gpui::transparent_black()
                            })
                            .text_color(if is_data { t.text_1 } else { t.text_2 })
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(ts) =
                                    this.active_tab_mut().and_then(|t| t.table_state.as_mut())
                                {
                                    ts.subview = TableSubview::Data;
                                }
                                cx.notify();
                            }))
                            .child("数据"),
                    )
                    .child(
                        div()
                            .id("subview-structure")
                            .px(px(10.0))
                            .py(px(4.0))
                            .rounded(px(4.0))
                            .text_size(t.fs_sm())
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .cursor_pointer()
                            .bg(if !is_data {
                                t.panel_bg
                            } else {
                                gpui::transparent_black()
                            })
                            .text_color(if !is_data { t.text_1 } else { t.text_2 })
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(ts) =
                                    this.active_tab_mut().and_then(|t| t.table_state.as_mut())
                                {
                                    ts.subview = TableSubview::Structure;
                                }
                                cx.notify();
                            }))
                            .child("表结构"),
                    ),
            )
            .when(is_data, |d| {
                let has_where = !ts.where_clause.is_empty() || !ts.applied_where_clause.is_empty();
                let filter_changed = ts.where_clause != ts.applied_where_clause;
                let refresh_tip_theme = *t;
                d.child(
                    div()
                        .flex_1()
                        .h(t.control_h())
                        .px(px(8.0))
                        .rounded(px(6.0))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_size(t.fs_xs())
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(t.purple)
                                .child("WHERE"),
                        )
                        .child(
                            div().flex_1().min_w_0().h_full().flex().items_center().child(
                                Input::new(&self.where_input)
                                    .appearance(false)
                                    .px_0()
                                    .py_0()
                                    .h_full()
                                    .text_size(t.fs_md()),
                            ),
                        )
                        .when(has_where, |bar| {
                            bar.child(
                                div()
                                    .id("clear-where-btn")
                                    .w(px(18.0))
                                    .h(px(18.0))
                                    .rounded(px(4.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.surface_2))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.where_input
                                            .update(cx, |s, cx| s.set_value("", window, cx));
                                        this.apply_table_filter(String::new(), cx);
                                    }))
                                    .child(hgi("cancel", 10.0, t.text_3)),
                            )
                        })
                        .child(
                            div()
                                .id("apply-where-btn")
                                .h(px(22.0))
                                .px(px(8.0))
                                .rounded(px(5.0))
                                .bg(if filter_changed { t.accent_soft } else { t.surface_2 })
                                .border_1()
                                .border_color(if filter_changed { t.accent } else { t.border })
                                .text_size(t.fs_sm())
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                .hover(|s| s.bg(t.surface_3))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let val = this.where_input.read(cx).value().to_string();
                                    this.apply_table_filter(val, cx);
                                }))
                                .child(if filter_changed { "应用筛选" } else { "筛选" }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .id("refresh-table-btn")
                                .h(t.control_h())
                                .w(t.control_h())
                                .rounded(px(6.0))
                                .bg(t.surface_2)
                                .border_1()
                                .border_color(t.border)
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(|s| s.bg(t.surface_3).border_color(t.border_strong))
                                .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                                .tooltip(move |window, cx| {
                                    build_sidebar_tooltip(
                                        "刷新当前表 (⌘R)".to_string(),
                                        refresh_tip_theme,
                                        window,
                                        cx,
                                    )
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.reload_active_table(cx);
                                }))
                                .child(hgi("refresh", 13.0, t.text_2)),
                        )
                        .child(self.toolbar_btn(
                            "cols-picker-btn",
                            "列显示",
                            t,
                            cx.listener(|this, _, _, cx| {
                                this.data.active_modal = ActiveModal::ColumnsPicker;
                                cx.notify();
                            }),
                        ))
                        .child(self.toolbar_btn(
                            "add-row-btn",
                            "+ 新增行",
                            t,
                            cx.listener(|this, _, _, cx| {
                                let Some(tab) = this.active_tab_mut() else {
                                    return;
                                };
                                let Some(ts) = tab.table_state.as_mut() else {
                                    return;
                                };
                                let new_row: Vec<Option<String>> = ts
                                    .columns
                                    .iter()
                                    .map(|c| c.default_expr.clone())
                                    .collect();
                                let row_idx = ts.rows.len();
                                let mut values = BTreeMap::new();
                                for (i, col) in ts.columns.iter().enumerate() {
                                    if !col.is_pk {
                                        values.insert(col.name.clone(), new_row[i].clone());
                                    }
                                }
                                let schema = ts.schema.clone();
                                let table = ts.table.clone();
                                ts.rows.push(new_row);
                                ts.added_rows.insert(row_idx);
                                ts.selected_row = row_idx;
                                this.data.staged_changes.push(StagedChange {
                                    schema,
                                    table,
                                    row_idx,
                                    pk_col: "id".into(),
                                    pk_val: None,
                                    kind: StagedChangeKind::Insert { values },
                                });
                                cx.notify();
                            }),
                        ))
                        .child(
                            div()
                                .id("del-row-btn")
                                .h(t.control_h())
                                .px(px(10.0))
                                .rounded(px(6.0))
                                .bg(t.red_soft)
                                .border_1()
                                .border_color(with_alpha(t.red, 0.3))
                                .text_color(t.red)
                                .text_size(t.fs_md())
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let Some(tab) = this.active_tab_mut() else {
                                        return;
                                    };
                                    let Some(ts) = tab.table_state.as_mut() else {
                                        return;
                                    };
                                    let targets: Vec<usize> = if !ts.checked_rows.is_empty() {
                                        ts.checked_rows.iter().copied().collect()
                                    } else {
                                        vec![ts.selected_row]
                                    };
                                    let pk_idx = ts.columns.iter().position(|c| c.is_pk).unwrap_or(0);
                                    let pk_col = ts
                                        .columns
                                        .get(pk_idx)
                                        .map(|c| c.name.clone())
                                        .unwrap_or_else(|| "id".into());
                                    let schema = ts.schema.clone();
                                    let table = ts.table.clone();
                                    let mut new_changes = Vec::new();
                                    let mut count = 0;
                                    for i in targets {
                                        if i < ts.rows.len() && !ts.deleted_rows.contains(&i) {
                                            ts.deleted_rows.insert(i);
                                            let pk_val = ts.rows[i].get(pk_idx).cloned().flatten();
                                            new_changes.push(StagedChange {
                                                schema: schema.clone(),
                                                table: table.clone(),
                                                row_idx: i,
                                                pk_col: pk_col.clone(),
                                                pk_val,
                                                kind: StagedChangeKind::Delete,
                                            });
                                            count += 1;
                                        }
                                    }
                                    ts.checked_rows.clear();
                                    this.data.staged_changes.extend(new_changes);
                                    this.show_toast(format!("已标记删除 {count} 行"), cx);
                                }))
                                .child("删除"),
                        )
                        .child(self.toolbar_btn(
                            "import-btn",
                            "导入",
                            t,
                            cx.listener(|this, _, _, cx| {
                                this.data.active_modal =
                                    ActiveModal::ExportImport { is_import: true };
                                cx.notify();
                            }),
                        ))
                        .child(self.toolbar_btn(
                            "export-btn",
                            "导出",
                            t,
                            cx.listener(|this, _, _, cx| {
                                this.data.active_modal =
                                    ActiveModal::ExportImport { is_import: false };
                                cx.notify();
                            }),
                        )),
                )
            });

        let body = if !is_data {
            self.render_structure_view(t, cx)
        } else {
            self.render_data_grid(t, cx)
        };

        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
            .child(subtoolbar)
            .when_some(ts.page_error.as_ref(), |d, error| {
                d.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .px(px(12.0))
                        .py(px(6.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .bg(t.red_soft)
                        .text_size(t.fs_md())
                        .text_color(t.red)
                        .child(div().flex_1().min_w_0().child(error.clone()))
                        .child(self.toolbar_btn(
                            "retry-table-page",
                            "重试",
                            t,
                            cx.listener(|this, _, _, cx| this.reload_active_table(cx)),
                        )),
                )
            })
            .child(body)
            .into_any_element()
    }

    fn toolbar_btn(
        &self,
        id: &'static str,
        label: &'static str,
        t: &VeloTheme,
        on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .h(t.control_h())
            .px(px(10.0))
            .rounded(px(6.0))
            .bg(t.surface_2)
            .border_1()
            .border_color(t.border)
            .text_color(t.text_1)
            .text_size(t.fs_md())
            .font_weight(gpui::FontWeight::MEDIUM)
            .flex()
            .items_center()
            .cursor_pointer()
            .hover(|s| s.bg(t.surface_3).border_color(t.border_strong))
            .on_click(on_click)
            .child(label)
            .into_any_element()
    }

    fn render_data_grid(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let Some(ts) = self.active_tab().and_then(|tab| tab.table_state.as_ref()) else {
            return div().into_any_element();
        };

        let mut layout = TableRowLayout {
            height: t.table_row_h(&self.data.settings.row_density),
            font_size: t.fs_md(),
            updated_columns: HashMap::new(),
        };
        for change in &self.data.staged_changes {
            if change.schema == ts.schema && change.table == ts.table {
                if let StagedChangeKind::Update { col, .. } = &change.kind {
                    layout
                        .updated_columns
                        .entry(change.row_idx)
                        .or_default()
                        .insert(col.clone());
                }
            }
        }
        let columns = Rc::new(GridColumns::new(
            px(ROW_NUMBER_WIDTH),
            ts.columns
                .iter()
                .enumerate()
                .filter(|(_, col)| !ts.hidden_cols.contains(&col.name))
                .map(|(i, col)| (i, px(column_width(ts, col)))),
        ));
        let total_width = columns.width;
        let theme = *t;
        let header_h = t.table_header_h();
        let header = self.grid_header(
            header_h,
            t,
            &columns,
            &ts.horizontal_scroll,
            cx,
            Self::table_header_cells,
        );
        let list = grid_rows(
            gpui::SharedString::from(format!("table-rows-{}", self.data.active_tab_id)),
            ts.rows.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                let Some(ts) = this.active_tab().and_then(|tab| tab.table_state.as_ref()) else {
                    return Vec::new();
                };
                let visible = columns.visible(&ts.horizontal_scroll, window);
                range
                    .map(|ri| this.render_table_row(ts, ri, &layout, visible, &theme, cx))
                    .collect()
            }),
        )
        .size_full()
        .track_scroll(&ts.vertical_scroll);

        let row_summary = if ts.is_loading {
            format!("正在加载第 {} 页…", ts.page + 1)
        } else if ts.page_error.is_some() {
            if ts.rows.is_empty() { "查询失败" } else { "查询失败 · 保留上次结果" }.to_string()
        } else if ts.checked_rows.is_empty() {
            format!("显示 {} 行 · {:.1} ms", ts.rows.len(), ts.elapsed_ms)
        } else {
            format!(
                "已选 {} 行 / 共 {} 行",
                ts.checked_rows.len(),
                ts.rows.len()
            )
        };
        let ready = !ts.is_loading && ts.page_error.is_none();
        let has_prev = ready && ts.page > 0;
        let has_next = ready && ts.has_next_page;
        let number = ts.page + 1;
        let page_label = if !ready {
            String::new()
        } else if !has_next {
            format!("第 {number} 页 · 末页")
        } else if ts.applied_where_clause.trim().is_empty() && ts.total_estimate > 0 {
            let estimate = (ts.total_estimate as usize).div_ceil(ts.limit).max(number + 1);
            format!("第 {number} 页 / 约 {estimate} 页")
        } else {
            format!("第 {number} 页")
        };
        let body = div()
            .flex_1()
            .min_h_0()
            .relative()
            .child(list)
            .when(ts.rows.is_empty(), |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .w_full()
                        .p(px(28.0))
                        .text_center()
                        .text_size(t.fs_md())
                        .text_color(t.text_3)
                        .child(if ts.is_loading {
                            "正在加载数据..."
                        } else if ts.page_error.is_some() {
                            "未能加载数据"
                        } else if !ts.applied_where_clause.trim().is_empty() {
                            "没有符合筛选条件的行"
                        } else {
                            "暂无数据行"
                        }),
                )
            });

        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .child(grid_viewport(
                &self.data.active_tab_id,
                total_width,
                header_h + px(1.0),
                header,
                body,
                &ts.vertical_scroll,
                &ts.horizontal_scroll,
            ))
            // Pagination footer
            .child(
                div()
                    .h(px(34.0))
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(t.border)
                    .bg(t.surface_1)
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(t.fs_sm())
                    .text_color(t.text_2)
                    .child(row_summary)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .id("cycle-limit-btn")
                                    .px(px(8.0))
                                    .py(px(2.0))
                                    .rounded(px(4.0))
                                    .bg(t.surface_2)
                                    .border_1()
                                    .border_color(t.border)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(ts) = this
                                            .active_tab_mut()
                                            .and_then(|t| t.table_state.as_mut())
                                        {
                                            ts.limit = match ts.limit {
                                                100 => 500,
                                                500 => 1000,
                                                _ => 100,
                                            };
                                            ts.page = 0;
                                            ts.vertical_scroll
                                                .scroll_to_item(0, gpui::ScrollStrategy::Top);
                                        }
                                        this.query_active_table(cx);
                                    }))
                                    .child(format!("LIMIT {}", ts.limit)),
                            )
                            .child(
                                div()
                                    .id("prev-page-btn")
                                    .h(px(22.0))
                                    .px(px(6.0))
                                    .rounded(px(4.0))
                                    .bg(t.surface_2)
                                    .border_1()
                                    .border_color(t.border)
                                    .flex()
                                    .items_center()
                                    .opacity(if has_prev { 1.0 } else { 0.4 })
                                    .when(has_prev, |d| {
                                        d.cursor_pointer().hover(|s| s.bg(t.surface_3))
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.change_table_page(false, cx)))
                                    .child("‹"),
                            )
                            .child(page_label)
                            .child(
                                div()
                                    .id("next-page-btn")
                                    .h(px(22.0))
                                    .px(px(6.0))
                                    .rounded(px(4.0))
                                    .bg(t.surface_2)
                                    .border_1()
                                    .border_color(t.border)
                                    .flex()
                                    .items_center()
                                    .opacity(if has_next { 1.0 } else { 0.4 })
                                    .when(has_next, |d| {
                                        d.cursor_pointer().hover(|s| s.bg(t.surface_3))
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.change_table_page(true, cx)))
                                    .child("›"),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn change_table_page(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(ts) = self.active_tab_mut().and_then(|tab| tab.table_state.as_mut()) else {
            return;
        };
        if ts.is_loading || ts.page_error.is_some() {
            return;
        }
        match forward {
            true if ts.has_next_page => ts.page += 1,
            false if ts.page > 0 => ts.page -= 1,
            _ => return,
        }
        ts.vertical_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        self.query_active_table(cx);
    }

    fn table_header_cells(
        &self,
        visible: VisibleColumns<'_>,
        t: &VeloTheme,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let Some(ts) = self.active_tab().and_then(|tab| tab.table_state.as_ref()) else {
            return div();
        };
        let header_h = t.table_header_h();
        let mut header_cells: Vec<AnyElement> = vec![div()
            .w(px(ROW_NUMBER_WIDTH))
            .h(header_h)
            .flex_shrink_0()
            .px(px(6.0))
            .border_r_1()
            .border_color(t.border)
            .flex()
            .items_center()
            .justify_center()
            .text_size(t.fs_sm())
            .text_color(t.text_3)
            .child("#")
            .into_any_element()];

        if visible.leading_space > px(0.0) {
            header_cells.push(
                div()
                    .w(visible.leading_space)
                    .flex_shrink_0()
                    .into_any_element(),
            );
        }
        for column in visible.cells {
            let ci = column.index;
            let col = &ts.columns[ci];
            let col_name = col.name.clone();
            let resize_col_name = col.name.clone();
            let reset_col_name = col.name.clone();
            let arrow = if ts.sort_col.as_deref() == Some(&col.name) {
                if ts.sort_asc {
                    " ↑"
                } else {
                    " ↓"
                }
            } else {
                ""
            };
            let is_pk = col.is_pk;
            let is_fk = col.fk_table.is_some();
            let w = column.width;
            let col_w_val = f32::from(w);
            let is_resizing_col = matches!(
                &self.data.col_resizing,
                Some(ColResizeState {
                    target: ColResizeTarget::Table { col_name },
                    ..
                }) if col_name == &col.name
            );
            let mut tip_parts = vec![format!("{} ({})", col.name, col.data_type)];
            if is_pk {
                tip_parts.push("PK".to_string());
            }
            if let Some(fk) = &col.fk_table {
                tip_parts.push(format!("FK → {fk}"));
            }
            if !col.nullable {
                tip_parts.push("NOT NULL".to_string());
            }
            let th_tip = tip_parts.join(" · ");
            let th_tip_theme = *t;

            header_cells.push(
                div()
                    .id(("th", ci))
                    .relative()
                    .w(w)
                    .h(header_h)
                    .flex_shrink_0()
                    .px(px(10.0))
                    .border_r_1()
                    .border_color(t.border)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.surface_2))
                    .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                    .tooltip(move |window, cx| {
                        build_sidebar_tooltip(th_tip.clone(), th_tip_theme, window, cx)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(ts) = this.active_tab_mut().and_then(|t| t.table_state.as_mut())
                        {
                            ts.page = 0;
                            ts.vertical_scroll
                                .scroll_to_item(0, gpui::ScrollStrategy::Top);
                            if ts.sort_col.as_deref() == Some(&col_name) {
                                ts.sort_asc = !ts.sort_asc;
                            } else {
                                ts.sort_col = Some(col_name.clone());
                                ts.sort_asc = true;
                            }
                        }
                        this.query_active_table(cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .text_size(t.fs_md())
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.text_1)
                            .when(is_pk, |d| {
                                d.child(div().flex_shrink_0().child(hgi("key", 12.0, t.amber)))
                            })
                            .when(!is_pk && is_fk, |d| {
                                d.child(div().flex_shrink_0().child(hgi("link-up", 12.0, t.accent)))
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(format!("{}{arrow}", col.name)),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .bg(t.surface_2)
                            .text_size(t.fs_xs())
                            .text_color(t.text_3)
                            .child(col.data_type.clone()),
                    )
                    .child(
                        div()
                            .id(("th-resize", ci))
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
                                        if let Some(ts) = this
                                            .active_tab_mut()
                                            .and_then(|t| t.table_state.as_mut())
                                        {
                                            ts.col_widths.remove(&reset_col_name);
                                        }
                                    } else {
                                        this.data.col_resizing = Some(ColResizeState {
                                            target: ColResizeTarget::Table {
                                                col_name: resize_col_name.clone(),
                                            },
                                            start_x: f32::from(ev.position.x),
                                            start_width: col_w_val,
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
                    .into_any_element(),
            );
        }
        div().flex().items_center().children(header_cells)
    }

    fn render_table_row(
        &self,
        ts: &TableTabState,
        ri: usize,
        layout: &TableRowLayout,
        visible: VisibleColumns<'_>,
        t: &VeloTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = &ts.rows[ri];
        let updated_columns = layout.updated_columns.get(&ri);
        let is_sel_row = ts.selected_row == ri;
        let is_checked = ts.checked_rows.contains(&ri);
        let is_added = ts.added_rows.contains(&ri);
        let is_deleted = ts.deleted_rows.contains(&ri);

        let row_bg = if is_deleted {
            t.red_soft
        } else if is_added {
            t.green_soft
        } else if is_checked {
            t.accent_soft
        } else if is_sel_row {
            t.row_selected
        } else {
            gpui::transparent_black()
        };

        let mut cells: Vec<AnyElement> = vec![div()
            .id(("rn", ri))
            .w(px(ROW_NUMBER_WIDTH))
            .h(layout.height)
            .flex_shrink_0()
            .px(px(6.0))
            .border_r_1()
            .border_color(t.border)
            .bg(t.surface_1)
            .flex()
            .items_center()
            .justify_center()
            .text_size(t.fs_sm())
            .text_color(if is_checked { t.accent } else { t.text_3 })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(ts) = this.active_tab_mut().and_then(|t| t.table_state.as_mut()) {
                    if ts.checked_rows.contains(&ri) {
                        ts.checked_rows.remove(&ri);
                    } else {
                        ts.checked_rows.insert(ri);
                    }
                }
                cx.notify();
            }))
            .child(if is_checked {
                "☑".to_string()
            } else {
                format!("{}", ri + 1)
            })
            .into_any_element()];

        if visible.leading_space > px(0.0) {
            cells.push(div().w(visible.leading_space).flex_shrink_0().into_any_element());
        }
        for column in visible.cells {
            let col_idx = column.index;
            let w = column.width;
            let col = &ts.columns[col_idx];
            let val = row.get(col_idx).and_then(|v| v.as_ref());
            let is_sel_cell = ts.selected_row == ri && ts.selected_col_idx == col_idx;
            let is_editing = ts.editing_cell == Some((ri, col_idx));
            let is_dirty = updated_columns.is_some_and(|cols| cols.contains(&col.name));
            let is_jsonb = is_json_type(&col.data_type);

            let cell_content: AnyElement = if is_editing {
                div()
                    .flex()
                    .items_center()
                    .w_full()
                    .h_full()
                    .min_w_0()
                    .px(px(6.0))
                    .bg(t.panel_bg)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .items_center()
                            .child(
                                Input::new(&self.inline_cell_input)
                                    .appearance(false)
                                    .px_0()
                                    .py_0()
                                    .h_full()
                                    .text_size(layout.font_size),
                            ),
                    )
                    .child(
                        div()
                            .id(("inline-commit", ri * 100 + col_idx))
                            .flex_shrink_0()
                            .px(px(4.0))
                            .text_size(t.fs_xs())
                            .text_color(t.accent)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let v = this.inline_cell_input.read(cx).value().to_string();
                                if let Some(ts) =
                                    this.active_tab_mut().and_then(|t| t.table_state.as_mut())
                                {
                                    ts.editing_cell = None;
                                }
                                this.stage_cell_update(ri, col_idx, Some(v), cx);
                            }))
                            .child("✓"),
                    )
                    .into_any_element()
            } else {
                match val {
                    None => div()
                        .w_full()
                        .min_w_0()
                        .truncate()
                        .text_color(t.text_3)
                        .italic()
                        .child(self.data.settings.null_label.clone())
                        .into_any_element(),
                    Some(s) if col.fk_table.is_some() && self.data.settings.show_fk_jump => {
                        let target = col.fk_table.clone().unwrap_or_default();
                        let fk_col = col.fk_column.clone().unwrap_or_else(|| "id".into());
                        div()
                            .id(("fk-jump", ri * 100 + col_idx))
                            .max_w_full()
                            .min_w_0()
                            .truncate()
                            .text_color(t.cyan)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                let Some(value) = this
                                    .active_tab()
                                    .and_then(|tab| tab.table_state.as_ref())
                                    .and_then(|ts| ts.rows.get(ri))
                                    .and_then(|row| row.get(col_idx))
                                    .and_then(Option::as_ref)
                                    .cloned()
                                else {
                                    return;
                                };
                                let where_expr = format!("{fk_col} = '{value}'");
                                let sch = this.data.current_schema.clone();
                                this.open_table_tab(&sch, &target, Some(where_expr), window, cx);
                                this.show_toast(
                                    format!("外键跳转 → {target} ({fk_col} = {value})"),
                                    cx,
                                );
                            }))
                            .child(format!("{} ↗", compact_cell_preview(s, 80)))
                            .into_any_element()
                    }
                    Some(s) if is_jsonb => {
                        let preview = compact_cell_preview(s, 80);
                        div()
                            .max_w_full()
                            .min_w_0()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(t.purple_soft)
                            .text_color(t.purple)
                            .text_size(t.fs_sm())
                            .truncate()
                            .child(preview)
                            .into_any_element()
                    }
                    Some(s) if col.name == "status" => {
                        let (bg, fg) = match s.as_str() {
                            "active" | "paid" => (t.green_soft, t.green),
                            "pending" | "idle" => (t.amber_soft, t.amber),
                            _ => (t.red_soft, t.red),
                        };
                        div()
                            .max_w_full()
                            .min_w_0()
                            .px(px(7.0))
                            .py(px(1.0))
                            .rounded(px(99.0))
                            .bg(bg)
                            .text_color(fg)
                            .text_size(t.fs_sm())
                            .truncate()
                            .child(format!("● {}", compact_cell_preview(s, 40)))
                            .into_any_element()
                    }
                    Some(s) => {
                        let display = if self.data.settings.thousand_sep
                            && (col.data_type == "int4" || col.data_type == "int8")
                        {
                            if let Ok(n) = s.parse::<i64>() {
                                format_thousands(n)
                            } else {
                                compact_cell_preview(s, 120)
                            }
                        } else {
                            compact_cell_preview(s, 120)
                        };
                        div()
                            .w_full()
                            .min_w_0()
                            .truncate()
                            .child(display)
                            .into_any_element()
                    }
                }
            };

            cells.push(
                div()
                    .id(("cell", ri * 10_000 + col_idx))
                    .relative()
                    .w(w)
                    .h(layout.height)
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .px(if is_editing { px(0.0) } else { px(10.0) })
                    .border_r_1()
                    .border_color(if is_dirty { t.amber } else { t.border })
                    .bg(if is_dirty {
                        t.amber_soft
                    } else {
                        gpui::transparent_black()
                    })
                    .text_size(layout.font_size)
                    .overflow_hidden()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
                        if is_editing {
                            return;
                        }
                        this.focus_handle.focus(window, cx);
                        if let Some(ts) = this.active_tab_mut().and_then(|t| t.table_state.as_mut())
                        {
                            ts.selected_row = ri;
                            ts.selected_col_idx = col_idx;
                            ts.editing_cell = None;
                        }
                        if ev.click_count() >= 2 {
                            this.data.drawer_open = true;
                            this.data.drawer_tab = DrawerTab::Cell;
                        }
                        this.sync_inspector_for_active_cell(window, cx);
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if let Some(ts) =
                                this.active_tab_mut().and_then(|t| t.table_state.as_mut())
                            {
                                ts.selected_row = ri;
                                ts.selected_col_idx = col_idx;
                            }
                            this.data.sidebar_ctx_menu.open = false;
                            this.data.tab_ctx_menu.open = false;
                            this.data.ctx_menu.open = true;
                            this.data.ctx_menu.is_sql = false;
                            this.data.ctx_menu.x = ev.position.x.into();
                            this.data.ctx_menu.y = ev.position.y.into();
                            this.data.ctx_menu.row_idx = ri;
                            this.data.ctx_menu.col_idx = col_idx;
                            cx.notify();
                        }),
                    )
                    .child(cell_content)
                    .when(is_sel_cell, |d| {
                        d.child(
                            div()
                                .absolute()
                                .inset_0()
                                .border_1()
                                .border_color(t.accent),
                        )
                    })
                    .into_any_element(),
            );
        }

        div()
            .h(layout.height)
            .flex_shrink_0()
            .flex()
            .items_center()
            .bg(row_bg)
            .border_b_1()
            .border_color(t.border)
            .hover(|s| s.bg(t.row_hover))
            .children(cells)
            .into_any_element()
    }

    pub fn render_staged_bar(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let count = self.data.staged_changes.len();
        if count == 0 {
            return div().into_any_element();
        }
        div()
            .absolute()
            .bottom(px(ROW_NUMBER_WIDTH))
            .left(px(180.0))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border_strong)
            .rounded(px(12.0))
            .px(px(12.0))
            .py(px(8.0))
            .shadow_lg()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .text_size(t.fs_md())
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(t.amber)
                    .child(format!("● {count} 项待提交修改")),
            )
            .child(self.toolbar_btn(
                "preview-sql-btn",
                "预览 SQL",
                t,
                cx.listener(|this, _, _, cx| {
                    this.data.active_modal = ActiveModal::PreviewSql;
                    cx.notify();
                }),
            ))
            .child(self.toolbar_btn(
                "discard-staged-btn",
                "丢弃",
                t,
                cx.listener(|this, _, _, cx| {
                    this.discard_staged_changes(cx);
                }),
            ))
            .child(
                div()
                    .id("commit-staged-btn")
                    .h(t.control_h())
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
                        let is_prod = this
                            .data
                            .active_conn
                            .as_ref()
                            .map(|c| c.env == "prod")
                            .unwrap_or(false);
                        if this.data.settings.safe_mode && is_prod {
                            this.data.active_modal = ActiveModal::PreviewSql;
                            cx.notify();
                        } else {
                            this.commit_staged_changes(cx);
                        }
                    }))
                    .child("提交修改"),
            )
            .into_any_element()
    }

    pub fn render_context_menu(
        &self,
        t: &VeloTheme,
        window: &gpui::Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cm = &self.data.ctx_menu;
        let row_idx = cm.row_idx;
        let col_idx = cm.col_idx;
        let is_sql = cm.is_sql;
        let vp = window.viewport_size();
        let max_x = (f32::from(vp.width) - 204.0).max(8.0);
        let max_y = (f32::from(vp.height) - 200.0).max(8.0);

        let item = |id: &'static str,
                    label: &'static str,
                    danger: bool,
                    on_click: Box<dyn Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App)>| {
            div()
                .id(id)
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(5.0))
                .text_size(t.fs_md())
                .text_color(if danger { t.red } else { t.text_1 })
                .cursor_pointer()
                .hover(|s| s.bg(t.accent).text_color(gpui::white()))
                .on_click(on_click)
                .child(label)
        };

        div()
            .id("cell-ctx-menu")
            .absolute()
            .left(px(cm.x.clamp(8.0, max_x)))
            .top(px(cm.y.clamp(8.0, max_y)))
            .w(px(190.0))
            .p(px(5.0))
            .rounded(px(9.0))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border_strong)
            .shadow_lg()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(item(
                "ctx-inspect",
                "在右侧检视器查看",
                false,
                Box::new(cx.listener(move |this, _, window, cx| {
                    this.data.ctx_menu.open = false;
                    this.data.drawer_open = true;
                    this.data.drawer_tab = DrawerTab::Cell;
                    this.sync_inspector_for_active_cell(window, cx);
                    cx.notify();
                })),
            ))
            .child(item(
                "ctx-copy",
                "复制单元格 (⌘C)",
                false,
                Box::new(cx.listener(move |this, _, _, cx| {
                    this.data.ctx_menu.open = false;
                    let val = if is_sql {
                        this.active_tab()
                            .and_then(|t| t.sql_state.as_ref())
                            .and_then(|ss| ss.result.as_ref())
                            .and_then(|r| r.rows.get(row_idx))
                            .and_then(|row| row.get(col_idx))
                            .and_then(|v| v.as_deref())
                            .unwrap_or("NULL")
                            .to_string()
                    } else {
                        this.active_tab()
                            .and_then(|t| t.table_state.as_ref())
                            .and_then(|ts| ts.rows.get(row_idx))
                            .and_then(|r| r.get(col_idx))
                            .and_then(|v| v.as_deref())
                            .unwrap_or("NULL")
                            .to_string()
                    };
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(val));
                    this.show_toast("已复制单元格值", cx);
                })),
            ))
            .child(item(
                "ctx-copy-json",
                "复制整行 JSON",
                false,
                Box::new(cx.listener(move |this, _, _, cx| {
                    this.data.ctx_menu.open = false;
                    let mut map = serde_json::Map::new();
                    if is_sql {
                        if let Some(res) = this
                            .active_tab()
                            .and_then(|t| t.sql_state.as_ref())
                            .and_then(|ss| ss.result.as_ref())
                        {
                            if let Some(row) = res.rows.get(row_idx) {
                                for (i, col_name) in res.columns.iter().enumerate() {
                                    let v = row
                                        .get(i)
                                        .and_then(|x| x.as_ref())
                                        .map(|s| serde_json::Value::String(s.clone()))
                                        .unwrap_or(serde_json::Value::Null);
                                    map.insert(col_name.clone(), v);
                                }
                            }
                        }
                    } else if let Some(ts) = this.active_tab().and_then(|t| t.table_state.as_ref())
                    {
                        if let Some(row) = ts.rows.get(row_idx) {
                            for (i, c) in ts.columns.iter().enumerate() {
                                let v = row
                                    .get(i)
                                    .and_then(|x| x.as_ref())
                                    .map(|s| serde_json::Value::String(s.clone()))
                                    .unwrap_or(serde_json::Value::Null);
                                map.insert(c.name.clone(), v);
                            }
                        }
                    }
                    if !map.is_empty() {
                        let json_str = serde_json::to_string_pretty(&map).unwrap_or_default();
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(json_str));
                        this.show_toast("已复制整行 JSON", cx);
                    }
                })),
            ))
            .when(!is_sql, |d| {
                d.child(div().h(px(1.0)).bg(t.border).my(px(4.0)))
                    .child(item(
                        "ctx-set-null",
                        "设为 NULL",
                        false,
                        Box::new(cx.listener(move |this, _, _, cx| {
                            this.data.ctx_menu.open = false;
                            this.stage_cell_update(row_idx, col_idx, None, cx);
                        })),
                    ))
                    .child(item(
                        "ctx-del-row",
                        "删除该行",
                        true,
                        Box::new(cx.listener(move |this, _, _, cx| {
                            this.data.ctx_menu.open = false;
                            if let Some(ts) =
                                this.active_tab_mut().and_then(|t| t.table_state.as_mut())
                            {
                                if row_idx < ts.rows.len() && !ts.deleted_rows.contains(&row_idx) {
                                    ts.deleted_rows.insert(row_idx);
                                    let pk_idx =
                                        ts.columns.iter().position(|c| c.is_pk).unwrap_or(0);
                                    let pk_col = ts
                                        .columns
                                        .get(pk_idx)
                                        .map(|c| c.name.clone())
                                        .unwrap_or_else(|| "id".into());
                                    let pk_val = ts.rows[row_idx].get(pk_idx).cloned().flatten();
                                    let schema = ts.schema.clone();
                                    let table = ts.table.clone();
                                    this.data.staged_changes.push(StagedChange {
                                        schema,
                                        table,
                                        row_idx,
                                        pk_col,
                                        pk_val,
                                        kind: StagedChangeKind::Delete,
                                    });
                                    this.show_toast("已标记删除该行", cx);
                                }
                            }
                            cx.notify();
                        })),
                    ))
            })
            .into_any_element()
    }
}

fn format_thousands(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 {
        out.push('-');
    }
    out.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_columns_keep_geometry_and_source_indices_across_a_scroll_jump() {
        // Column 1 is hidden, column 3 was resized. The gutter always precedes column 0.
        let widths = [(0, 90.), (2, 60.), (3, 280.), (4, 80.), (5, 70.), (6, 100.)];
        let columns = GridColumns::new(px(44.), widths.into_iter().map(|(i, w)| (i, px(w))));
        assert_eq!(columns.width, px(724.));
        let first = columns.visible_at(px(0.), px(134.));
        assert_eq!(
            first.cells.iter().map(|c| c.index).collect::<Vec<_>>(),
            [0, 2]
        );
        assert_eq!(first.leading_space, px(0.));

        let last = columns.visible_at(px(554.), px(170.));
        assert_eq!(
            last.cells.iter().map(|c| c.index).collect::<Vec<_>>(),
            [4, 5, 6]
        );
        assert_eq!(last.leading_space + px(44.), px(474.));
        let partial = columns.visible_at(px(430.), px(80.));
        assert_eq!(
            partial.cells.iter().map(|c| c.index).collect::<Vec<_>>(),
            [2, 3, 4, 5]
        );
    }

    #[test]
    fn empty_grid_keeps_its_gutter_and_a_wide_viewport_includes_every_column() {
        let empty = GridColumns::new(px(44.), std::iter::empty());
        assert_eq!(empty.width, px(44.));
        assert!(empty.visible_at(px(0.), px(800.)).cells.is_empty());
        let columns = GridColumns::new(px(0.), (0..3).map(|i| (i, px(160.))));
        let visible = columns.visible_at(px(0.), px(800.));
        assert_eq!(visible.cells.len(), 3);
        assert_eq!(visible.leading_space, px(0.));
    }
}
