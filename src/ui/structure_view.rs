use crate::adapter::postgres::format_sql;
use crate::ui::app::VeloDbApp;
use crate::ui::icons::hgi;
use crate::ui::state::StructTab;
use crate::ui::theme::VeloTheme;
use gpui::{
    div, prelude::*, px, AnyElement, Context, InteractiveElement, ParentElement, Styled,
};
use gpui_component::input::Editor;
use gpui_component::scroll::ScrollableElement;

/// Split SQL text into top-level statements separated by `;`, ignoring semicolons
/// inside string literals, quoted identifiers, or comments.
fn split_ddl_statements(sql: &str) -> Vec<&str> {
    let bytes = sql.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut quote: Option<u8> = None;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while i < bytes.len() {
        let b = bytes[i];
        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                in_block_comment = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if let Some(q) = quote {
            if b == b'\\' && q != b'`' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == q {
                if i + 1 < bytes.len() && bytes[i + 1] == q {
                    i += 2;
                    continue;
                }
                quote = None;
            }
            i += 1;
            continue;
        }

        match b {
            b'\'' | b'"' | b'`' => quote = Some(b),
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => in_line_comment = true,
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                in_block_comment = true;
                i += 2;
                continue;
            }
            b';' => {
                let piece = sql[start..i].trim();
                if !piece.is_empty() {
                    out.push(piece);
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }

    let tail = sql[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Collapse runs of ASCII whitespace outside quotes/comments into single spaces.
fn normalize_inline_ws(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    let mut quote: Option<u8> = None;
    let mut prev_space = true;

    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            if b == b'\\' && q != b'`' && i + 1 < bytes.len() {
                let next_ch = s[i + 1..].chars().next().unwrap();
                out.push(next_ch);
                i += 1 + next_ch.len_utf8();
                continue;
            }
            if b == q {
                if i + 1 < bytes.len() && bytes[i + 1] == q {
                    out.push(q as char);
                    i += 2;
                    continue;
                }
                quote = None;
            }
            i += ch.len_utf8();
            prev_space = false;
            continue;
        }

        if matches!(b, b'\'' | b'"' | b'`') {
            quote = Some(b);
            out.push(b as char);
            i += 1;
            prev_space = false;
            continue;
        }

        let ch = s[i..].chars().next().unwrap();
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
        i += ch.len_utf8();
    }

    out.trim().to_string()
}

/// Split top-level comma-separated clauses inside `CREATE TABLE (...)`.
fn split_top_level_commas(body: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let mut items = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while i < bytes.len() {
        let b = bytes[i];
        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                in_block_comment = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if let Some(q) = quote {
            if b == b'\\' && q != b'`' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == q {
                if i + 1 < bytes.len() && bytes[i + 1] == q {
                    i += 2;
                    continue;
                }
                quote = None;
            }
            i += 1;
            continue;
        }

        match b {
            b'\'' | b'"' | b'`' => quote = Some(b),
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => in_line_comment = true,
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                in_block_comment = true;
                i += 2;
                continue;
            }
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                let item = normalize_inline_ws(&body[start..i]);
                if !item.is_empty() {
                    items.push(item);
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }

    let last = normalize_inline_ws(&body[start..]);
    if !last.is_empty() {
        items.push(last);
    }
    items
}

/// Tokenize MySQL/Postgres table tail options into logical option clauses.
fn split_table_options(tail: &str) -> Vec<String> {
    let bytes = tail.as_bytes();
    let mut raw_tokens = Vec::new();
    let mut start = None;
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;

    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' && q != b'`' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == q {
                if i + 1 < bytes.len() && bytes[i + 1] == q {
                    i += 2;
                    continue;
                }
                quote = None;
            }
            i += 1;
            continue;
        }

        match b {
            b'\'' | b'"' | b'`' => {
                if start.is_none() {
                    start = Some(i);
                }
                quote = Some(b);
            }
            b'(' => {
                if start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
            }
            b' ' | b'\t' | b'\n' | b'\r' if depth == 0 => {
                if let Some(s) = start.take() {
                    raw_tokens.push(tail[s..i].to_string());
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
        i += 1;
    }
    if let Some(s) = start {
        raw_tokens.push(tail[s..].to_string());
    }

    let mut clauses = Vec::new();
    let mut idx = 0usize;
    while idx < raw_tokens.len() {
        let tok = &raw_tokens[idx];
        let upper = tok.to_ascii_uppercase();
        if upper == "DEFAULT" && idx + 1 < raw_tokens.len() {
            let next_upper = raw_tokens[idx + 1].to_ascii_uppercase();
            if next_upper.starts_with("CHARACTER") && idx + 2 < raw_tokens.len() {
                clauses.push(format!(
                    "{} {} {}",
                    tok,
                    raw_tokens[idx + 1],
                    raw_tokens[idx + 2]
                ));
                idx += 3;
                continue;
            }
            clauses.push(format!("{} {}", tok, raw_tokens[idx + 1]));
            idx += 2;
            continue;
        }
        if upper == "CHARACTER" && idx + 2 < raw_tokens.len() {
            clauses.push(format!(
                "{} {} {}",
                tok,
                raw_tokens[idx + 1],
                raw_tokens[idx + 2]
            ));
            idx += 3;
            continue;
        }
        if (upper == "PARTITION" || upper == "SUBPARTITION") && idx + 1 < raw_tokens.len() {
            clauses.push(raw_tokens[idx..].join(" "));
            break;
        }
        if idx + 2 < raw_tokens.len() && raw_tokens[idx + 1] == "=" {
            clauses.push(format!("{}={}", tok, raw_tokens[idx + 2]));
            idx += 3;
            continue;
        }
        clauses.push(tok.clone());
        idx += 1;
    }
    clauses
}

/// Find the byte range `(open_paren_idx, close_paren_idx)` of the first top-level `(...)`.
fn find_first_paren_block(stmt: &str) -> Option<(usize, usize)> {
    let bytes = stmt.as_bytes();
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut open_idx = None;
    let mut quote: Option<u8> = None;

    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' && q != b'`' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == q {
                if i + 1 < bytes.len() && bytes[i + 1] == q {
                    i += 2;
                    continue;
                }
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'\'' | b'"' | b'`' => quote = Some(b),
            b'(' => {
                if depth == 0 {
                    open_idx = Some(i);
                }
                depth += 1;
            }
            b')' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        return open_idx.map(|o| (o, i));
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn is_create_table_stmt(stmt: &str) -> bool {
    let prefix = normalize_inline_ws(stmt).to_ascii_uppercase();
    prefix.starts_with("CREATE TABLE ")
        || prefix.starts_with("CREATE TEMPORARY TABLE ")
        || prefix.starts_with("CREATE UNLOGGED TABLE ")
}

/// Format a database DDL string (MySQL `SHOW CREATE TABLE` or PostgreSQL DDL)
/// with consistent indentation, readable table options, and trailing semicolons.
pub fn format_ddl(raw: &str, indent_spaces: u8) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("--") && !trimmed.contains('\n') {
        return trimmed.to_string();
    }

    let indent = " ".repeat(indent_spaces.clamp(2, 8) as usize);
    let stmts = split_ddl_statements(trimmed);
    if stmts.is_empty() {
        return trimmed.to_string();
    }

    let mut formatted_stmts = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        let mut comment_lines = Vec::new();
        let mut body_lines = Vec::new();
        let mut past_leading_comments = false;
        for line in stmt.lines() {
            let l = line.trim();
            if !past_leading_comments && (l.starts_with("--") || l.is_empty()) {
                if !l.is_empty() {
                    comment_lines.push(l.to_string());
                }
            } else {
                past_leading_comments = true;
                body_lines.push(line);
            }
        }
        let body = body_lines.join("\n").trim().to_string();
        if body.is_empty() {
            if !comment_lines.is_empty() {
                formatted_stmts.push(comment_lines.join("\n"));
            }
            continue;
        }

        let formatted_body = if is_create_table_stmt(&body) {
            if let Some((open_idx, close_idx)) = find_first_paren_block(&body) {
                let header = normalize_inline_ws(&body[..open_idx]);
                let inner = &body[open_idx + 1..close_idx];
                let tail = normalize_inline_ws(&body[close_idx + 1..]);
                let items = split_top_level_commas(inner);

                let mut out = format!("{header} (\n");
                for (ix, item) in items.iter().enumerate() {
                    out.push_str(&indent);
                    out.push_str(item);
                    if ix + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push(')');

                if !tail.is_empty() {
                    let options = split_table_options(&tail);
                    if options.len() <= 1 {
                        out.push(' ');
                        out.push_str(&tail);
                    } else {
                        for (ix, opt) in options.iter().enumerate() {
                            if ix == 0 {
                                out.push(' ');
                                out.push_str(opt);
                            } else {
                                out.push('\n');
                                out.push_str(&indent);
                                out.push_str(opt);
                            }
                        }
                    }
                }
                out.push(';');
                out
            } else {
                format!("{};", normalize_inline_ws(&body))
            }
        } else {
            let mut f = format_sql(&body, true, indent_spaces.clamp(2, 8));
            if !f.ends_with(';') {
                f.push(';');
            }
            f
        };

        if comment_lines.is_empty() {
            formatted_stmts.push(formatted_body);
        } else {
            formatted_stmts.push(format!("{}\n{}", comment_lines.join("\n"), formatted_body));
        }
    }

    formatted_stmts.join("\n\n")
}

impl VeloDbApp {
    pub fn render_structure_view(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let Some(ts) = self.active_tab().and_then(|tab| tab.table_state.as_ref()) else {
            return div().into_any_element();
        };
        let s_tab = ts.struct_tab;
        let ddl_formatted = ts.ddl_formatted;
        let ddl_soft_wrap = ts.ddl_soft_wrap;
        let idx_count = ts
            .structure
            .as_ref()
            .map(|s| s.indexes.len())
            .unwrap_or(0);
        let has_ddl = ts
            .structure
            .as_ref()
            .is_some_and(|s| !s.ddl.is_empty());
        let target_full_table = if ts.schema.is_empty() {
            ts.table.clone()
        } else {
            format!("{}.{}", ts.schema, ts.table)
        };

        let seg_btn = |id: &'static str, label: String, target: StructTab| {
            let active = s_tab == target;
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
                    if let Some(ts) = this.active_tab_mut().and_then(|t| t.table_state.as_mut()) {
                        ts.struct_tab = target;
                    }
                    cx.notify();
                }))
                .child(label)
        };

        let body: AnyElement = match (&ts.structure, s_tab) {
            (Some(st), StructTab::Columns) => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                // Sticky columns header
                .child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .bg(t.titlebar_bg)
                        .border_b_1()
                        .border_color(t.border_strong)
                        .px(px(10.0))
                        .py(px(7.0))
                        .text_size(t.fs_md())
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(div().w(px(180.0)).flex_shrink_0().child("字段名"))
                        .child(div().w(px(150.0)).flex_shrink_0().child("数据类型"))
                        .child(div().w(px(80.0)).flex_shrink_0().child("可空"))
                        .child(div().w(px(180.0)).flex_shrink_0().child("默认值"))
                        .child(div().flex_1().min_w_0().child("备注")),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_y_scrollbar()
                        .flex()
                        .flex_col()
                        .children(st.columns.iter().map(|c| {
                            let is_pk = c.is_pk;
                            let is_fk = c.fk_table.is_some();
                            div()
                                .flex()
                                .items_center()
                                .px(px(10.0))
                                .py(px(7.0))
                                .border_b_1()
                                .border_color(t.border)
                                .hover(|s| s.bg(t.row_hover))
                                .text_size(t.fs_md())
                                .child(
                                    div()
                                        .w(px(180.0))
                                        .flex_shrink_0()
                                        .pr(px(8.0))
                                        .flex()
                                        .items_center()
                                        .gap(px(5.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .when(is_pk, |d| {
                                            d.child(
                                                div()
                                                    .flex_shrink_0()
                                                    .child(hgi("key", 12.0, t.amber)),
                                            )
                                        })
                                        .when(!is_pk && is_fk, |d| {
                                            d.child(
                                                div()
                                                    .flex_shrink_0()
                                                    .child(hgi("link-up", 12.0, t.accent)),
                                            )
                                        })
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .child(c.name.clone()),
                                        ),
                                )
                                .child(
                                    div().w(px(150.0)).flex_shrink_0().pr(px(8.0)).child(
                                        div()
                                            .max_w_full()
                                            .px(px(5.0))
                                            .py(px(1.0))
                                            .rounded(px(4.0))
                                            .bg(t.surface_2)
                                            .text_color(t.text_2)
                                            .text_size(t.fs_xs())
                                            .truncate()
                                            .child(c.data_type.clone()),
                                    ),
                                )
                                .child(
                                    div()
                                        .w(px(80.0))
                                        .flex_shrink_0()
                                        .text_color(t.text_2)
                                        .child(if c.nullable { "YES" } else { "NO" }),
                                )
                                .child(
                                    div()
                                        .w(px(180.0))
                                        .flex_shrink_0()
                                        .pr(px(8.0))
                                        .truncate()
                                        .text_color(t.text_2)
                                        .child(
                                            c.default_expr
                                                .clone()
                                                .unwrap_or_else(|| "-".into()),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(t.text_3)
                                        .child(c.comment.clone().unwrap_or_default()),
                                )
                        })),
                )
                .into_any_element(),

            (Some(st), StructTab::Indexes) => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                // Sticky indexes header
                .child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .bg(t.titlebar_bg)
                        .border_b_1()
                        .border_color(t.border_strong)
                        .px(px(10.0))
                        .py(px(7.0))
                        .text_size(t.fs_md())
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(div().w(px(210.0)).flex_shrink_0().child("索引名"))
                        .child(div().w(px(160.0)).flex_shrink_0().child("字段"))
                        .child(div().w(px(90.0)).flex_shrink_0().child("类型"))
                        .child(div().w(px(70.0)).flex_shrink_0().child("唯一"))
                        .child(div().w(px(90.0)).flex_shrink_0().child("体积"))
                        .child(div().flex_1().min_w_0().child("扫描次数")),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_y_scrollbar()
                        .flex()
                        .flex_col()
                        .children(st.indexes.iter().map(|x| {
                            div()
                                .flex()
                                .items_center()
                                .px(px(10.0))
                                .py(px(7.0))
                                .border_b_1()
                                .border_color(t.border)
                                .hover(|s| s.bg(t.row_hover))
                                .text_size(t.fs_md())
                                .child(
                                    div()
                                        .w(px(210.0))
                                        .flex_shrink_0()
                                        .pr(px(8.0))
                                        .truncate()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(x.name.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(160.0))
                                        .flex_shrink_0()
                                        .pr(px(8.0))
                                        .truncate()
                                        .child(x.columns.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(90.0))
                                        .flex_shrink_0()
                                        .text_color(t.text_2)
                                        .child(x.method.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(70.0))
                                        .flex_shrink_0()
                                        .child(if x.is_unique { "YES" } else { "NO" }),
                                )
                                .child(div().w(px(90.0)).flex_shrink_0().child(x.size.clone()))
                                .child(div().flex_1().min_w_0().text_color(t.green).child(x.scans.clone()))
                        })),
                )
                .into_any_element(),

            (Some(st), StructTab::Constraints) => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                // Sticky constraints header
                .child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .bg(t.titlebar_bg)
                        .border_b_1()
                        .border_color(t.border_strong)
                        .px(px(10.0))
                        .py(px(7.0))
                        .text_size(t.fs_md())
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(div().w(px(210.0)).flex_shrink_0().child("约束名"))
                        .child(div().w(px(130.0)).flex_shrink_0().child("类型"))
                        .child(div().flex_1().min_w_0().child("定义")),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_y_scrollbar()
                        .flex()
                        .flex_col()
                        .children(st.constraints.iter().map(|c| {
                            div()
                                .flex()
                                .items_center()
                                .px(px(10.0))
                                .py(px(7.0))
                                .border_b_1()
                                .border_color(t.border)
                                .hover(|s| s.bg(t.row_hover))
                                .text_size(t.fs_md())
                                .child(
                                    div()
                                        .w(px(210.0))
                                        .flex_shrink_0()
                                        .pr(px(8.0))
                                        .truncate()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(c.name.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(130.0))
                                        .flex_shrink_0()
                                        .text_color(t.accent)
                                        .child(c.constraint_type.clone()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_color(t.text_2)
                                        .child(c.definition.clone()),
                                )
                        })),
                )
                .into_any_element(),

            (Some(_), StructTab::Ddl) => div()
                .flex_1()
                .min_h_0()
                .p(px(12.0))
                .child(
                    div()
                        .size_full()
                        .rounded(px(8.0))
                        .bg(t.panel_bg)
                        .border_1()
                        .border_color(t.border)
                        .overflow_hidden()
                        .child(
                            Editor::new(&ts.ddl_editor)
                                .h_full()
                                .appearance(false)
                                .bordered(false)
                                .readonly(true)
                                .text_size(t.fs_md()),
                        ),
                )
                .into_any_element(),

            (None, _) => div()
                .p(px(20.0))
                .text_color(t.text_3)
                .child("正在加载表结构...")
                .into_any_element(),
        };

        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
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
                            .p(px(2.0))
                            .rounded(px(6.0))
                            .bg(t.surface_2)
                            .border_1()
                            .border_color(t.border)
                            .child(seg_btn("st-cols", "字段".into(), StructTab::Columns))
                            .child(seg_btn(
                                "st-idxs",
                                format!("索引 ({idx_count})"),
                                StructTab::Indexes,
                            ))
                            .child(seg_btn(
                                "st-cons",
                                "约束与外键".into(),
                                StructTab::Constraints,
                            ))
                            .child(seg_btn("st-ddl", "DDL".into(), StructTab::Ddl)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .when(s_tab == StructTab::Ddl && has_ddl, |d| {
                                d.child(
                                    div()
                                        .id("ddl-format-toggle")
                                        .h(px(28.0))
                                        .px(px(10.0))
                                        .rounded(px(6.0))
                                        .bg(if ddl_formatted { t.accent_soft } else { t.surface_2 })
                                        .border_1()
                                        .border_color(if ddl_formatted { t.accent } else { t.border })
                                        .text_color(if ddl_formatted { t.accent } else { t.text_2 })
                                        .text_size(t.fs_md())
                                        .flex()
                                        .items_center()
                                        .gap(px(5.0))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(if ddl_formatted { t.accent_soft } else { t.surface_3 }))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            let tab_size = this.data.settings.tab_size;
                                            let Some(ts) = this.active_tab_mut().and_then(|t| t.table_state.as_mut()) else {
                                                return;
                                            };
                                            ts.ddl_formatted = !ts.ddl_formatted;
                                            let next_text = ts
                                                .structure
                                                .as_ref()
                                                .map(|st| {
                                                    if ts.ddl_formatted {
                                                        format_ddl(&st.ddl, tab_size)
                                                    } else {
                                                        st.ddl.clone()
                                                    }
                                                })
                                                .unwrap_or_default();
                                            ts.ddl_editor.update(cx, |ed, cx| {
                                                ed.set_value(next_text, window, cx);
                                            });
                                            cx.notify();
                                        }))
                                        .child(hgi("magic", 12.0, if ddl_formatted { t.accent } else { t.text_2 }))
                                        .child(if ddl_formatted { "已格式化" } else { "原始 DDL" }),
                                )
                                .child(
                                    div()
                                        .id("ddl-wrap-toggle")
                                        .h(px(28.0))
                                        .px(px(10.0))
                                        .rounded(px(6.0))
                                        .bg(if ddl_soft_wrap { t.accent_soft } else { t.surface_2 })
                                        .border_1()
                                        .border_color(if ddl_soft_wrap { t.accent } else { t.border })
                                        .text_color(if ddl_soft_wrap { t.accent } else { t.text_2 })
                                        .text_size(t.fs_md())
                                        .flex()
                                        .items_center()
                                        .cursor_pointer()
                                        .hover(|s| s.bg(if ddl_soft_wrap { t.accent_soft } else { t.surface_3 }))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            let Some(ts) = this.active_tab_mut().and_then(|t| t.table_state.as_mut()) else {
                                                return;
                                            };
                                            ts.ddl_soft_wrap = !ts.ddl_soft_wrap;
                                            let wrap = ts.ddl_soft_wrap;
                                            ts.ddl_editor.update(cx, |ed, cx| {
                                                ed.set_soft_wrap(wrap, window, cx);
                                            });
                                            cx.notify();
                                        }))
                                        .child("换行"),
                                )
                                .child(
                                    div()
                                        .id("copy-ddl-btn")
                                        .h(px(28.0))
                                        .px(px(10.0))
                                        .rounded(px(6.0))
                                        .bg(t.surface_2)
                                        .border_1()
                                        .border_color(t.border)
                                        .text_color(t.text_1)
                                        .text_size(t.fs_md())
                                        .flex()
                                        .items_center()
                                        .cursor_pointer()
                                        .hover(|s| s.bg(t.surface_3))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            let Some(ts) = this.active_tab().and_then(|tab| tab.table_state.as_ref()) else {
                                                return;
                                            };
                                            let ddl = ts.ddl_editor.read(cx).text().to_string();
                                            if ddl.is_empty() {
                                                return;
                                            }
                                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(ddl));
                                            this.show_toast("DDL 已复制到剪贴板", cx);
                                        }))
                                        .child("复制 DDL"),
                                )
                            })
                            .child(
                                div()
                                    .id("add-col-draft-btn")
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
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let draft = format!(
                                            "ALTER TABLE {target_full_table} ADD COLUMN new_column VARCHAR(255);"
                                        );
                                        this.add_sql_tab_with_sql(draft, window, cx);
                                        this.show_toast("已在 SQL 标签页打开 ALTER TABLE 草稿", cx);
                                    }))
                                    .child(hgi("plus", 13.0, gpui::white()))
                                    .child("新增字段"),
                            ),
                    ),
            )
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::format_ddl;

    #[test]
    fn formats_mysql_create_table_with_options() {
        let raw = "CREATE TABLE `activity` (`id` bigint(20) NOT NULL AUTO_INCREMENT, `owner_id` bigint(20) NOT NULL COMMENT 'activity owner, test', `content` json NOT NULL COMMENT 'activity content', PRIMARY KEY (`id`)) ENGINE=InnoDB AUTO_INCREMENT=764 DEFAULT CHARSET=utf8 COMMENT='activity'";
        let formatted = format_ddl(raw, 2);
        assert_eq!(
            formatted,
            "\
CREATE TABLE `activity` (
  `id` bigint(20) NOT NULL AUTO_INCREMENT,
  `owner_id` bigint(20) NOT NULL COMMENT 'activity owner, test',
  `content` json NOT NULL COMMENT 'activity content',
  PRIMARY KEY (`id`)
) ENGINE=InnoDB
  AUTO_INCREMENT=764
  DEFAULT CHARSET=utf8
  COMMENT='activity';"
        );
    }

    #[test]
    fn formats_postgres_create_table_and_indexes() {
        let raw = "CREATE TABLE \"public\".\"users\" (\n  \"id\" bigint NOT NULL DEFAULT nextval('users_id_seq'::regclass),\n  \"meta\" jsonb\n);\n\nCREATE INDEX \"idx_meta\" ON \"public\".\"users\" USING gin (meta);";
        let formatted = format_ddl(raw, 2);
        assert!(formatted.contains("CREATE TABLE \"public\".\"users\" (\n  \"id\""));
        assert!(formatted.contains("\n);"));
        assert!(formatted.contains("CREATE INDEX \"idx_meta\""));
    }
}
