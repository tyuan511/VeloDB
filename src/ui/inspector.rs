use crate::config::save_settings;
use crate::ui::app::VeloDbApp;
use crate::ui::data_grid::compact_cell_preview;
use crate::ui::icons::hgi;
use crate::ui::sidebar::{build_sidebar_tooltip, SIDEBAR_TOOLTIP_DELAY};
use crate::ui::state::{DrawerTab, InspectorViewMode, TabKind};
use crate::ui::theme::VeloTheme;
use gpui::{
    div, prelude::*, px, AnyElement, Context, InteractiveElement, MouseButton, ParentElement,
    Styled,
};
use gpui_component::input::Editor;
use gpui_component::scroll::ScrollableElement;
use std::collections::HashSet;

fn is_json_col_type(col_type: &str) -> bool {
    let dt = col_type.trim();
    dt.eq_ignore_ascii_case("json") || dt.eq_ignore_ascii_case("jsonb")
}

#[derive(Debug)]
enum InspectorContentState {
    Null,
    ValidJson {
        value: serde_json::Value,
        summary: String,
        byte_len: usize,
    },
    InvalidJson {
        line: usize,
        col: usize,
        message: String,
        byte_len: usize,
    },
    PlainText {
        chars: usize,
        lines: usize,
    },
}

fn json_value_summary(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Object(map) => format!("Object · {} keys", map.len()),
        serde_json::Value::Array(arr) => format!("Array · {} items", arr.len()),
        serde_json::Value::String(s) => format!("String · {} chars", s.chars().count()),
        serde_json::Value::Number(_) => "Number".to_string(),
        serde_json::Value::Bool(_) => "Boolean".to_string(),
        serde_json::Value::Null => "null".to_string(),
    }
}

fn analyze_inspector_content(raw: &str, col_type: &str, is_null: bool) -> InspectorContentState {
    if is_null {
        return InspectorContentState::Null;
    }
    let is_json_col = is_json_col_type(col_type);
    let trimmed = raw.trim_start();
    let looks_like_json = trimmed.starts_with('{') || trimmed.starts_with('[');

    if (is_json_col || looks_like_json) && !raw.trim().is_empty() {
        match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(value) => InspectorContentState::ValidJson {
                summary: json_value_summary(&value),
                value,
                byte_len: raw.len(),
            },
            Err(err) => InspectorContentState::InvalidJson {
                line: err.line(),
                col: err.column(),
                message: err.to_string(),
                byte_len: raw.len(),
            },
        }
    } else {
        InspectorContentState::PlainText {
            chars: raw.chars().count(),
            lines: raw.lines().count().max(1),
        }
    }
}

fn format_byte_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}

fn inspector_value_modified(
    original: Option<&str>,
    current_raw: &str,
    current_is_null: bool,
) -> bool {
    match (original, current_is_null) {
        (None, true) => false,
        (None, false) => true,
        (Some(_), true) => true,
        (Some(orig), false) => {
            if orig == current_raw {
                return false;
            }
            if let (Ok(v1), Ok(v2)) = (
                serde_json::from_str::<serde_json::Value>(orig),
                serde_json::from_str::<serde_json::Value>(current_raw),
            ) {
                if matches!(v1, serde_json::Value::Object(_) | serde_json::Value::Array(_))
                    || matches!(v2, serde_json::Value::Object(_) | serde_json::Value::Array(_))
                {
                    return v1 != v2;
                }
            }
            true
        }
    }
}

#[derive(Clone, Debug)]
enum JsonTreeKind {
    Object(usize),
    Array(usize),
    String(String),
    Number(String),
    Bool(bool),
    Null,
}

#[derive(Clone, Debug)]
struct JsonTreeRow {
    path: String,
    depth: usize,
    key: Option<String>,
    kind: JsonTreeKind,
    copy_text: String,
    is_collapsed: bool,
}

fn flatten_json_tree(
    val: &serde_json::Value,
    collapsed: &HashSet<String>,
    max_rows: usize,
) -> (Vec<JsonTreeRow>, bool) {
    fn walk(
        val: &serde_json::Value,
        path: String,
        depth: usize,
        key: Option<String>,
        collapsed: &HashSet<String>,
        out: &mut Vec<JsonTreeRow>,
        max_rows: usize,
        truncated: &mut bool,
    ) {
        if out.len() >= max_rows {
            *truncated = true;
            return;
        }
        let is_collapsed = collapsed.contains(&path);
        match val {
            serde_json::Value::Object(map) => {
                let copy_text =
                    serde_json::to_string_pretty(val).unwrap_or_else(|_| "{}".to_string());
                out.push(JsonTreeRow {
                    path: path.clone(),
                    depth,
                    key,
                    kind: JsonTreeKind::Object(map.len()),
                    copy_text,
                    is_collapsed,
                });
                if !is_collapsed {
                    for (k, v) in map {
                        let child_path = format!("{path}.{k}");
                        walk(
                            v,
                            child_path,
                            depth + 1,
                            Some(k.clone()),
                            collapsed,
                            out,
                            max_rows,
                            truncated,
                        );
                        if *truncated {
                            break;
                        }
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                let copy_text =
                    serde_json::to_string_pretty(val).unwrap_or_else(|_| "[]".to_string());
                out.push(JsonTreeRow {
                    path: path.clone(),
                    depth,
                    key,
                    kind: JsonTreeKind::Array(arr.len()),
                    copy_text,
                    is_collapsed,
                });
                if !is_collapsed {
                    for (ix, v) in arr.iter().enumerate() {
                        let child_path = format!("{path}[{ix}]");
                        walk(
                            v,
                            child_path,
                            depth + 1,
                            Some(format!("[{ix}]")),
                            collapsed,
                            out,
                            max_rows,
                            truncated,
                        );
                        if *truncated {
                            break;
                        }
                    }
                }
            }
            serde_json::Value::String(s) => {
                out.push(JsonTreeRow {
                    path,
                    depth,
                    key,
                    kind: JsonTreeKind::String(s.clone()),
                    copy_text: s.clone(),
                    is_collapsed: false,
                });
            }
            serde_json::Value::Number(n) => {
                let text = n.to_string();
                out.push(JsonTreeRow {
                    path,
                    depth,
                    key,
                    kind: JsonTreeKind::Number(text.clone()),
                    copy_text: text,
                    is_collapsed: false,
                });
            }
            serde_json::Value::Bool(b) => {
                out.push(JsonTreeRow {
                    path,
                    depth,
                    key,
                    kind: JsonTreeKind::Bool(*b),
                    copy_text: b.to_string(),
                    is_collapsed: false,
                });
            }
            serde_json::Value::Null => {
                out.push(JsonTreeRow {
                    path,
                    depth,
                    key,
                    kind: JsonTreeKind::Null,
                    copy_text: "null".to_string(),
                    is_collapsed: false,
                });
            }
        }
    }

    let mut out = Vec::new();
    let mut truncated = false;
    walk(
        val,
        "$".to_string(),
        0,
        None,
        collapsed,
        &mut out,
        max_rows,
        &mut truncated,
    );
    (out, truncated)
}

fn collect_collapsible_paths(val: &serde_json::Value, path: &str, out: &mut HashSet<String>) {
    match val {
        serde_json::Value::Object(map) => {
            if !map.is_empty() {
                out.insert(path.to_string());
            }
            for (k, v) in map {
                collect_collapsible_paths(v, &format!("{path}.{k}"), out);
            }
        }
        serde_json::Value::Array(arr) => {
            if !arr.is_empty() {
                out.insert(path.to_string());
            }
            for (ix, v) in arr.iter().enumerate() {
                collect_collapsible_paths(v, &format!("{path}[{ix}]"), out);
            }
        }
        _ => {}
    }
}

impl VeloDbApp {
    pub fn render_inspector(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let Some(active_tab) = self.active_tab() else {
            return div().into_any_element();
        };
        let is_table = active_tab.kind == TabKind::Table;
        let is_sql = active_tab.kind == TabKind::Sql;
        if !is_table && !is_sql {
            return div().into_any_element();
        }

        let is_cell = self.data.drawer_tab == DrawerTab::Cell;
        let inspector_w = self.data.settings.inspector_width.clamp(260.0, 900.0);
        let resizing = self.data.inspector_resizing;
        let conn_read_only = self.data.active_conn.as_ref().is_some_and(|c| c.read_only);

        let (header_label, col_type, sel_r, sel_c, can_stage, row_title, column_count, orig_val) =
            if is_table {
                let Some(ts) = active_tab.table_state.as_ref() else {
                    return div().into_any_element();
                };
                let col_def = ts.columns.get(ts.selected_col_idx);
                let col_type = col_def
                    .map(|c| c.data_type.clone())
                    .unwrap_or_else(|| "text".into());
                let col_name = col_def
                    .map(|c| format!("{}.{} (row #{})", ts.table, c.name, ts.selected_row + 1))
                    .unwrap_or_else(|| "-".into());
                let orig = active_tab
                    .cell_value(ts.selected_row, ts.selected_col_idx)
                    .map(str::to_owned);

                (
                    col_name,
                    col_type,
                    ts.selected_row,
                    ts.selected_col_idx,
                    !ts.rows.is_empty() && !ts.columns.is_empty() && !conn_read_only,
                    format!("{} · 第 {} 行", ts.table, ts.selected_row + 1),
                    ts.columns.len(),
                    orig,
                )
            } else {
                let Some(ss) = active_tab.sql_state.as_ref() else {
                    return div().into_any_element();
                };
                let sel_r = ss.selected_row.unwrap_or(0);
                let sel_c = ss.selected_col_idx.unwrap_or(0);
                if let Some(res) = &ss.result {
                    let col_name = res
                        .columns
                        .get(sel_c)
                        .map(|c| format!("{} (row #{})", c, sel_r + 1))
                        .unwrap_or_else(|| "未选中单元格".into());
                    let orig = active_tab.cell_value(sel_r, sel_c).map(str::to_owned);

                    (
                        col_name,
                        "SQL".into(),
                        sel_r,
                        sel_c,
                        false,
                        format!("{} · 第 {} 行", active_tab.title, sel_r + 1),
                        res.columns.len(),
                        orig,
                    )
                } else {
                    (
                        "暂无查询结果".into(),
                        "SQL".into(),
                        0,
                        0,
                        false,
                        "暂无查询结果".into(),
                        0,
                        None,
                    )
                }
            };

        let body: AnyElement = if is_cell {
            let current_raw = self.inspector_editor_input.read(cx).text().to_string();
            let is_null = self.data.inspector_is_null;
            let content_state = analyze_inspector_content(&current_raw, &col_type, is_null);
            let is_valid_json = matches!(content_state, InspectorContentState::ValidJson { .. });
            let is_modified =
                can_stage && inspector_value_modified(orig_val.as_deref(), &current_raw, is_null);
            let view_mode = if is_valid_json {
                self.data.inspector_view_mode
            } else {
                InspectorViewMode::Editor
            };
            let soft_wrap = self.data.inspector_soft_wrap;

            let mini_btn = |id: &'static str,
                            label: &'static str,
                            active: bool,
                            disabled: bool| {
                div()
                    .id(id)
                    .h(px(24.0))
                    .px(px(8.0))
                    .rounded(px(5.0))
                    .bg(if active { t.accent_soft } else { t.surface_2 })
                    .border_1()
                    .border_color(if active { t.accent } else { t.border })
                    .text_size(t.fs_sm())
                    .text_color(if disabled {
                        t.text_3
                    } else if active {
                        t.accent
                    } else {
                        t.text_2
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!disabled, |d| {
                        d.cursor_pointer().hover(move |s| {
                            if active {
                                s.bg(t.accent_soft)
                            } else {
                                s.bg(t.surface_3).text_color(t.text_1)
                            }
                        })
                    })
                    .child(label)
            };

            let status_bar = match &content_state {
                InspectorContentState::Null => div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(t.fs_sm())
                    .text_color(t.text_3)
                    .child("当前值为 SQL NULL")
                    .into_any_element(),
                InspectorContentState::ValidJson {
                    summary, byte_len, ..
                } => div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .text_size(t.fs_sm())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(t.green)
                            .child(format!("✓ 合法 JSON · {summary}")),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(t.text_3)
                            .child(format_byte_size(*byte_len)),
                    )
                    .into_any_element(),
                InspectorContentState::InvalidJson {
                    line,
                    col,
                    message,
                    byte_len,
                } => {
                    let tip = message.clone();
                    let tip_theme = *t;
                    div()
                        .id("insp-json-err-status")
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .px(px(7.0))
                        .py(px(3.0))
                        .rounded(px(5.0))
                        .bg(t.red_soft)
                        .text_size(t.fs_sm())
                        .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                        .tooltip(move |window, cx| {
                            build_sidebar_tooltip(tip.clone(), tip_theme, window, cx)
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(t.red)
                                .child(format!("✗ JSON 错误 (L{line}:C{col}): {message}")),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(t.text_3)
                                .child(format_byte_size(*byte_len)),
                        )
                        .into_any_element()
                }
                InspectorContentState::PlainText { chars, lines } => div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(t.fs_sm())
                    .text_color(t.text_3)
                    .child(format!("纯文本 · {chars} 字符 · {lines} 行"))
                    .into_any_element(),
            };

            let main_viewer: AnyElement = match (view_mode, &content_state) {
                (InspectorViewMode::Tree, InspectorContentState::ValidJson { value, .. }) => {
                    let (rows, truncated) =
                        flatten_json_tree(value, &self.data.inspector_collapsed_paths, 800);
                    div()
                        .flex_1()
                        .min_h(px(160.0))
                        .rounded(px(9.0))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scrollbar()
                                .py(px(6.0))
                                .px(px(6.0))
                                .flex()
                                .flex_col()
                                .children(rows.into_iter().enumerate().map(|(ix, row)| {
                                    let is_container = matches!(
                                        row.kind,
                                        JsonTreeKind::Object(_) | JsonTreeKind::Array(_)
                                    );
                                    let path_for_toggle = row.path.clone();
                                    let copy_val = row.copy_text.clone();
                                    let indent_px = (row.depth as f32) * 14.0;

                                    let value_el: AnyElement = match &row.kind {
                                        JsonTreeKind::Object(len) => div()
                                            .text_color(t.text_3)
                                            .child(format!("{{ {len} keys }}"))
                                            .into_any_element(),
                                        JsonTreeKind::Array(len) => div()
                                            .text_color(t.text_3)
                                            .child(format!("[ {len} items ]"))
                                            .into_any_element(),
                                        JsonTreeKind::String(s) => div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_color(t.green)
                                            .child(format!("\"{}\"", compact_cell_preview(s, 180)))
                                            .into_any_element(),
                                        JsonTreeKind::Number(n) => div()
                                            .text_color(t.amber)
                                            .child(n.clone())
                                            .into_any_element(),
                                        JsonTreeKind::Bool(b) => div()
                                            .text_color(t.cyan)
                                            .child(if *b { "true" } else { "false" })
                                            .into_any_element(),
                                        JsonTreeKind::Null => div()
                                            .text_color(t.text_3)
                                            .child("null")
                                            .into_any_element(),
                                    };

                                    div()
                                        .id(("json-tree-row", ix))
                                        .group("json-tree-row")
                                        .w_full()
                                        .min_h(px(23.0))
                                        .pl(px(4.0 + indent_px))
                                        .pr(px(4.0))
                                        .py(px(2.0))
                                        .rounded(px(4.0))
                                        .flex()
                                        .items_center()
                                        .gap(px(4.0))
                                        .text_size(t.fs_md())
                                        .hover(|s| s.bg(t.row_hover))
                                        .when(is_container, |d| {
                                            d.cursor_pointer().on_click(cx.listener(
                                                move |this, _, _, cx| {
                                                    if !this
                                                        .data
                                                        .inspector_collapsed_paths
                                                        .remove(&path_for_toggle)
                                                    {
                                                        this.data
                                                            .inspector_collapsed_paths
                                                            .insert(path_for_toggle.clone());
                                                    }
                                                    cx.notify();
                                                },
                                            ))
                                        })
                                        .child(if is_container {
                                            div()
                                                .w(px(14.0))
                                                .flex_shrink_0()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .child(hgi(
                                                    if row.is_collapsed {
                                                        "arrow-right"
                                                    } else {
                                                        "arrow-down"
                                                    },
                                                    10.0,
                                                    t.text_3,
                                                ))
                                                .into_any_element()
                                        } else {
                                            div().w(px(14.0)).flex_shrink_0().into_any_element()
                                        })
                                        .when_some(row.key.as_ref(), |d, k| {
                                            d.child(
                                                div()
                                                    .flex_shrink_0()
                                                    .max_w(px(160.0))
                                                    .truncate()
                                                    .text_color(t.purple)
                                                    .font_weight(gpui::FontWeight::MEDIUM)
                                                    .child(k.clone()),
                                            )
                                            .child(div().flex_shrink_0().text_color(t.text_3).child(":"))
                                        })
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .flex()
                                                .items_center()
                                                .child(value_el),
                                        )
                                        .child(
                                            div()
                                                .id(("json-tree-copy", ix))
                                                .opacity(0.0)
                                                .group_hover("json-tree-row", |s| s.opacity(1.0))
                                                .flex_shrink_0()
                                                .px(px(5.0))
                                                .py(px(1.0))
                                                .rounded(px(3.0))
                                                .bg(t.surface_2)
                                                .border_1()
                                                .border_color(t.border)
                                                .text_size(t.fs_xs())
                                                .text_color(t.text_2)
                                                .cursor_pointer()
                                                .hover(|s| s.bg(t.surface_3).text_color(t.text_1))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    cx.write_to_clipboard(
                                                        gpui::ClipboardItem::new_string(
                                                            copy_val.clone(),
                                                        ),
                                                    );
                                                    this.show_toast("已复制节点值", cx);
                                                }))
                                                .child("复制"),
                                        )
                                }))
                                .when(truncated, |d| {
                                    d.child(
                                        div()
                                            .px(px(8.0))
                                            .py(px(6.0))
                                            .text_size(t.fs_sm())
                                            .text_color(t.text_3)
                                            .child("仅展示前 800 个节点，请折叠节点或切换到源码查看全部"),
                                    )
                                }),
                        )
                        .into_any_element()
                }
                _ => div()
                    .flex_1()
                    .min_h(px(160.0))
                    .rounded(px(9.0))
                    .bg(t.panel_bg)
                    .border_1()
                    .border_color(if matches!(
                        content_state,
                        InspectorContentState::InvalidJson { .. }
                    ) {
                        t.red
                    } else {
                        t.border
                    })
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        Editor::new(&self.inspector_editor_input)
                            .h_full()
                            .appearance(false)
                            .bordered(false)
                            .readonly(!can_stage)
                            .text_size(t.fs_md()),
                    )
                    .into_any_element(),
            };

            div()
                .flex_1()
                .min_h_0()
                .p(px(12.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_size(t.fs_sm())
                                .text_color(t.text_3)
                                .child(if can_stage { "当前单元格" } else { "只读预览" }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .when(is_null, |d| {
                                    d.child(
                                        div()
                                            .px(px(5.0))
                                            .py(px(1.0))
                                            .rounded(px(4.0))
                                            .bg(t.surface_2)
                                            .text_size(t.fs_xs())
                                            .text_color(t.text_3)
                                            .child("NULL"),
                                    )
                                })
                                .when(is_modified, |d| {
                                    d.child(
                                        div()
                                            .px(px(5.0))
                                            .py(px(1.0))
                                            .rounded(px(4.0))
                                            .bg(t.amber_soft)
                                            .text_size(t.fs_xs())
                                            .text_color(t.amber)
                                            .child("已修改"),
                                    )
                                })
                                .child(
                                    div()
                                        .px(px(5.0))
                                        .py(px(1.0))
                                        .rounded(px(4.0))
                                        .bg(t.surface_2)
                                        .text_size(t.fs_xs())
                                        .text_color(t.text_3)
                                        .child(col_type.clone()),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .w_full()
                        .min_w_0()
                        .truncate()
                        .text_size(t.fs_md())
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(header_label),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(6.0))
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
                                        .id("insp-mode-editor")
                                        .px(px(8.0))
                                        .py(px(2.0))
                                        .rounded(px(4.0))
                                        .text_size(t.fs_sm())
                                        .cursor_pointer()
                                        .bg(if view_mode == InspectorViewMode::Editor {
                                            t.panel_bg
                                        } else {
                                            gpui::transparent_black()
                                        })
                                        .text_color(if view_mode == InspectorViewMode::Editor {
                                            t.text_1
                                        } else {
                                            t.text_2
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.data.inspector_view_mode =
                                                InspectorViewMode::Editor;
                                            cx.notify();
                                        }))
                                        .child("源码"),
                                )
                                .child(
                                    div()
                                        .id("insp-mode-tree")
                                        .px(px(8.0))
                                        .py(px(2.0))
                                        .rounded(px(4.0))
                                        .text_size(t.fs_sm())
                                        .bg(if view_mode == InspectorViewMode::Tree {
                                            t.panel_bg
                                        } else {
                                            gpui::transparent_black()
                                        })
                                        .text_color(if !is_valid_json {
                                            t.text_3
                                        } else if view_mode == InspectorViewMode::Tree {
                                            t.text_1
                                        } else {
                                            t.text_2
                                        })
                                        .when(is_valid_json, |d| {
                                            d.cursor_pointer().on_click(cx.listener(
                                                |this, _, _, cx| {
                                                    this.data.inspector_view_mode =
                                                        InspectorViewMode::Tree;
                                                    cx.notify();
                                                },
                                            ))
                                        })
                                        .child("树形"),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .when(view_mode == InspectorViewMode::Editor, |d| {
                                    d.child(
                                        mini_btn(
                                            "format-json-btn",
                                            "格式化",
                                            false,
                                            !can_stage && !is_valid_json,
                                        )
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            let raw = this
                                                .inspector_editor_input
                                                .read(cx)
                                                .text()
                                                .to_string();
                                            match serde_json::from_str::<serde_json::Value>(&raw)
                                                .and_then(|v| serde_json::to_string_pretty(&v))
                                            {
                                                Ok(pretty) => {
                                                    this.data.inspector_is_null = false;
                                                    this.inspector_editor_input.update(
                                                        cx,
                                                        |s, cx| {
                                                            s.set_highlighter("json", cx);
                                                            s.set_value(pretty, window, cx);
                                                        },
                                                    );
                                                    this.refresh_inspector_diagnostics(cx);
                                                    this.show_toast("JSON 已格式化", cx);
                                                }
                                                Err(_) => {
                                                    this.show_toast("当前内容不是合法 JSON", cx)
                                                }
                                            }
                                        })),
                                    )
                                    .child(
                                        mini_btn(
                                            "minify-json-btn",
                                            "压缩",
                                            false,
                                            !can_stage,
                                        )
                                        .when(can_stage, |b| {
                                            b.on_click(cx.listener(|this, _, window, cx| {
                                                let raw = this
                                                    .inspector_editor_input
                                                    .read(cx)
                                                    .text()
                                                    .to_string();
                                                match serde_json::from_str::<serde_json::Value>(&raw)
                                                    .and_then(|v| serde_json::to_string(&v))
                                                {
                                                    Ok(compact) => {
                                                        this.data.inspector_is_null = false;
                                                        this.inspector_editor_input.update(
                                                            cx,
                                                            |s, cx| {
                                                                s.set_highlighter("json", cx);
                                                                s.set_value(compact, window, cx);
                                                            },
                                                        );
                                                        this.refresh_inspector_diagnostics(cx);
                                                        this.show_toast("JSON 已压缩", cx);
                                                    }
                                                    Err(_) => {
                                                        this.show_toast("当前内容不是合法 JSON", cx)
                                                    }
                                                }
                                            }))
                                        }),
                                    )
                                    .child(
                                        mini_btn("insp-wrap-btn", "换行", soft_wrap, false)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.data.inspector_soft_wrap =
                                                    !this.data.inspector_soft_wrap;
                                                let wrap = this.data.inspector_soft_wrap;
                                                this.inspector_editor_input.update(
                                                    cx,
                                                    |ed, cx| ed.set_soft_wrap(wrap, window, cx),
                                                );
                                                cx.notify();
                                            })),
                                    )
                                })
                                .when(view_mode == InspectorViewMode::Tree, |d| {
                                    d.child(
                                        mini_btn("tree-expand-all-btn", "展开", false, false)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.data.inspector_collapsed_paths.clear();
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        mini_btn("tree-collapse-all-btn", "折叠", false, false)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let raw = this
                                                    .inspector_editor_input
                                                    .read(cx)
                                                    .text()
                                                    .to_string();
                                                if let Ok(val) =
                                                    serde_json::from_str::<serde_json::Value>(&raw)
                                                {
                                                    let mut all = HashSet::new();
                                                    collect_collapsible_paths(&val, "$", &mut all);
                                                    all.remove("$");
                                                    this.data.inspector_collapsed_paths = all;
                                                    cx.notify();
                                                }
                                            })),
                                    )
                                })
                                .child(mini_btn("copy-insp-btn", "复制", false, false).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        let val = if this.data.inspector_is_null {
                                            "NULL".to_string()
                                        } else {
                                            this.inspector_editor_input.read(cx).text().to_string()
                                        };
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(val));
                                        this.show_toast("已复制检视器内容", cx);
                                    }),
                                )),
                        ),
                )
                .child(main_viewer)
                .child(div().flex_shrink_0().child(status_bar))
                .when(can_stage, |d| {
                    let stage_col_type = col_type.clone();
                    d.child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .id("apply-insp-btn")
                                    .flex_1()
                                    .h(t.control_h())
                                    .rounded(px(6.0))
                                    .bg(t.accent)
                                    .text_color(gpui::white())
                                    .text_size(t.fs_md())
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.data.inspector_is_null {
                                            this.stage_cell_update(sel_r, sel_c, None, cx);
                                            this.show_toast("已设为 NULL 并加入暂存区", cx);
                                            return;
                                        }
                                        let val = this
                                            .inspector_editor_input
                                            .read(cx)
                                            .text()
                                            .to_string();
                                        if is_json_col_type(&stage_col_type) {
                                            match serde_json::from_str::<serde_json::Value>(&val)
                                                .and_then(|v| serde_json::to_string(&v))
                                            {
                                                Ok(compact) => {
                                                    this.stage_cell_update(
                                                        sel_r,
                                                        sel_c,
                                                        Some(compact),
                                                        cx,
                                                    );
                                                    this.show_toast("单元格修改已加入暂存区", cx);
                                                }
                                                Err(_) => {
                                                    this.show_toast(
                                                        "JSON 语法有误，请先修正后再应用",
                                                        cx,
                                                    );
                                                }
                                            }
                                        } else {
                                            let next = match serde_json::from_str::<serde_json::Value>(
                                                &val,
                                            ) {
                                                Ok(
                                                    v @ (serde_json::Value::Object(_)
                                                    | serde_json::Value::Array(_)),
                                                ) => serde_json::to_string(&v).unwrap_or(val),
                                                _ => val,
                                            };
                                            this.stage_cell_update(sel_r, sel_c, Some(next), cx);
                                            this.show_toast("单元格修改已加入暂存区", cx);
                                        }
                                    }))
                                    .child("应用到暂存区"),
                            )
                            .when(!is_null, |b| {
                                b.child(
                                    div()
                                        .id("insp-set-null-btn")
                                        .h(t.control_h())
                                        .px(px(10.0))
                                        .rounded(px(6.0))
                                        .bg(t.surface_2)
                                        .border_1()
                                        .border_color(t.border)
                                        .text_size(t.fs_md())
                                        .text_color(t.text_2)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|s| s.bg(t.red_soft).text_color(t.red))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.stage_cell_update(sel_r, sel_c, None, cx);
                                            this.sync_inspector_for_active_cell(window, cx);
                                            this.show_toast("已设为 NULL 并加入暂存区", cx);
                                        }))
                                        .child("设为 NULL"),
                                )
                            })
                            .when(is_modified, |b| {
                                b.child(
                                    div()
                                        .id("insp-revert-btn")
                                        .h(t.control_h())
                                        .px(px(10.0))
                                        .rounded(px(6.0))
                                        .bg(t.surface_2)
                                        .border_1()
                                        .border_color(t.border)
                                        .text_size(t.fs_md())
                                        .text_color(t.text_2)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|s| s.bg(t.surface_3).text_color(t.text_1))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.sync_inspector_for_active_cell(window, cx);
                                            this.show_toast("已还原为当前单元格内容", cx);
                                        }))
                                        .child("还原"),
                                )
                            }),
                    )
                })
                .into_any_element()
        } else {
            let fields = (0..column_count).filter_map(|ci| {
                let (fname, ftype) = if let Some(ts) = &active_tab.table_state {
                    let col = ts.columns.get(ci)?;
                    (col.name.clone(), col.data_type.clone())
                } else {
                    let res = active_tab.sql_state.as_ref()?.result.as_ref()?;
                    (res.columns.get(ci)?.clone(), "result".to_owned())
                };
                let raw = active_tab.cell_value(sel_r, ci);
                let is_null = raw.is_none();
                let is_json = is_json_col_type(&ftype)
                    || raw.is_some_and(|s| {
                        let t = s.trim_start();
                        (t.starts_with('{') || t.starts_with('['))
                            && serde_json::from_str::<serde_json::Value>(s).is_ok()
                    });
                let preview = raw
                    .map(|s| compact_cell_preview(s, 140))
                    .unwrap_or_else(|| self.data.settings.null_label.clone());
                let is_cur_col = ci == sel_c;

                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(6.0))
                                .text_size(t.fs_sm())
                                .text_color(t.text_2)
                                .child(div().flex_1().min_w_0().truncate().child(fname))
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .gap(px(5.0))
                                        .when(is_json, |d| {
                                            d.child(
                                                div()
                                                    .px(px(4.0))
                                                    .py(px(0.5))
                                                    .rounded(px(3.0))
                                                    .bg(t.accent_soft)
                                                    .text_size(t.fs_xs())
                                                    .text_color(t.accent)
                                                    .child("JSON"),
                                            )
                                        })
                                        .child(div().text_color(t.text_3).child(ftype))
                                        .child(
                                            div()
                                                .id(("row-form-copy", ci))
                                                .px(px(5.0))
                                                .py(px(1.0))
                                                .rounded(px(3.0))
                                                .bg(t.surface_2)
                                                .border_1()
                                                .border_color(t.border)
                                                .text_size(t.fs_xs())
                                                .text_color(t.text_3)
                                                .cursor_pointer()
                                                .hover(|s| s.bg(t.surface_3).text_color(t.text_1))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    let value = this
                                                        .active_tab()
                                                        .and_then(|tab| tab.cell_value(sel_r, ci))
                                                        .unwrap_or("NULL");
                                                    cx.write_to_clipboard(
                                                        gpui::ClipboardItem::new_string(
                                                            value.to_owned(),
                                                        ),
                                                    );
                                                    this.show_toast("已复制字段值", cx);
                                                }))
                                                .child("复制"),
                                        )
                                        .when(can_stage, |d| {
                                            d.child(
                                                div()
                                                    .id(("row-form-null", ci))
                                                    .px(px(5.0))
                                                    .py(px(1.0))
                                                    .rounded(px(3.0))
                                                    .bg(t.surface_2)
                                                    .border_1()
                                                    .border_color(t.border)
                                                    .text_size(t.fs_xs())
                                                    .text_color(t.text_3)
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(t.red_soft).text_color(t.red))
                                                    .on_click(cx.listener(move |this, _, window, cx| {
                                                        cx.stop_propagation();
                                                        this.stage_cell_update(sel_r, ci, None, cx);
                                                        this.sync_inspector_for_active_cell(
                                                            window, cx,
                                                        );
                                                    }))
                                                    .child("NULL"),
                                            )
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .id(("row-form-val", ci))
                                .w_full()
                                .min_w_0()
                                .px(px(9.0))
                                .py(px(6.0))
                                .rounded(px(6.0))
                                .bg(t.panel_bg)
                                .border_1()
                                .border_color(if is_cur_col { t.accent } else { t.border })
                                .text_size(t.fs_md())
                                .text_color(if is_null { t.text_3 } else { t.text_1 })
                                .cursor_pointer()
                                .hover(|s| s.border_color(t.accent))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if is_table {
                                        if let Some(ts) = this
                                            .active_tab_mut()
                                            .and_then(|t| t.table_state.as_mut())
                                        {
                                            ts.selected_col_idx = ci;
                                        }
                                    } else if let Some(ss) =
                                        this.active_tab_mut().and_then(|t| t.sql_state.as_mut())
                                    {
                                        ss.selected_col_idx = Some(ci);
                                    }
                                    this.sync_inspector_for_active_cell(window, cx);
                                    this.data.drawer_tab = DrawerTab::Cell;
                                    cx.notify();
                                }))
                                .child(div().w_full().min_w_0().truncate().child(preview)),
                        )
                        .into_any_element(),
                )
            });

            div()
                .flex_1()
                .w_full()
                .min_w_0()
                .overflow_y_scrollbar()
                .p(px(12.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .text_size(t.fs_sm())
                        .text_color(t.text_3)
                        .child(row_title),
                )
                .children(fields)
                .into_any_element()
        };

        div()
            .id("inspector-panel")
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .w(px(inspector_w))
            .occlude()
            .bg(t.sidebar_bg)
            .border_l_1()
            .border_color(t.border_strong)
            .shadow_lg()
            .flex()
            .flex_col()
            .min_h_0()
            .child(
                div()
                    .px(px(12.0))
                    .py(px(9.0))
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
                            .child(
                                div()
                                    .id("drawer-cell-tab")
                                    .px(px(10.0))
                                    .py(px(4.0))
                                    .rounded(px(4.0))
                                    .text_size(t.fs_sm())
                                    .cursor_pointer()
                                    .bg(if is_cell {
                                        t.panel_bg
                                    } else {
                                        gpui::transparent_black()
                                    })
                                    .text_color(if is_cell { t.text_1 } else { t.text_2 })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.data.drawer_tab = DrawerTab::Cell;
                                        cx.notify();
                                    }))
                                    .child("单元格检视"),
                            )
                            .child(
                                div()
                                    .id("drawer-row-tab")
                                    .px(px(10.0))
                                    .py(px(4.0))
                                    .rounded(px(4.0))
                                    .text_size(t.fs_sm())
                                    .cursor_pointer()
                                    .bg(if !is_cell {
                                        t.panel_bg
                                    } else {
                                        gpui::transparent_black()
                                    })
                                    .text_color(if !is_cell { t.text_1 } else { t.text_2 })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.data.drawer_tab = DrawerTab::Row;
                                        cx.notify();
                                    }))
                                    .child("行表单"),
                            ),
                    )
                    .child(
                        div()
                            .id("close-drawer-btn")
                            .w(px(26.0))
                            .h(px(26.0))
                            .rounded(px(6.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(t.text_2)
                            .cursor_pointer()
                            .hover(|s| s.bg(t.surface_2))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.data.drawer_open = false;
                                cx.notify();
                            }))
                            .child(hgi("cancel", 14.0, t.text_2)),
                    ),
            )
            .child(body)
            // Left-edge draggable resize handle (double-click to reset width)
            .child(
                div()
                    .id("inspector-resize-handle")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(-3.0))
                    .w(px(6.0))
                    .cursor_col_resize()
                    .flex()
                    .justify_center()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &gpui::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if ev.click_count >= 2 {
                                this.data.inspector_resizing = false;
                                this.data.settings.inspector_width = 360.0;
                                save_settings(&this.data.settings);
                            } else {
                                this.data.inspector_resizing = true;
                            }
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .w(px(2.0))
                            .h_full()
                            .bg(if resizing {
                                t.accent
                            } else {
                                gpui::transparent_black()
                            })
                            .hover(|s| s.bg(t.accent)),
                    ),
            )
            .into_any_element()
    }
}
