use crate::adapter::{SchemaObject, SchemaObjectType};
use crate::ui::app::VeloDbApp;
use crate::ui::icons::hgi;
use crate::ui::state::{TabKind, TableDangerAction, TableSubview};
use crate::ui::theme::VeloTheme;
use gpui::{
    deferred, div, prelude::*, px, uniform_list, AnyElement, AnyView, App, Context, InteractiveElement,
    MouseButton, ParentElement, Styled, Window,
};
use gpui_component::input::Input;
use gpui_component::scroll::{ScrollableElement, Scrollbar};
use gpui_component::tooltip::Tooltip;
use gpui_component::InteractiveElementExt;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

pub(crate) const SIDEBAR_TOOLTIP_DELAY: Duration = Duration::from_millis(250);
pub(crate) const FAVORITES_GROUP_NAME: &str = "收藏";

#[derive(Clone)]
struct DraggedTab {
    tab_id: String,
    from_idx: usize,
    title: String,
    icon: &'static str,
    theme: VeloTheme,
}

struct DraggedTabView {
    title: String,
    icon: &'static str,
    theme: VeloTheme,
}

impl gpui::Render for DraggedTabView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        div()
            .h(t.control_h())
            .max_w(px(200.0))
            .px(px(10.0))
            .rounded(px(6.0))
            .flex()
            .items_center()
            .gap(px(7.0))
            .text_size(t.fs_md())
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.accent)
            .text_color(t.text_1)
            .font_weight(gpui::FontWeight::MEDIUM)
            .shadow_md()
            .opacity(0.92)
            .child(
                div()
                    .flex_shrink_0()
                    .opacity(0.85)
                    .child(hgi(self.icon, 13.0, t.text_1)),
            )
            .child(div().min_w_0().truncate().child(self.title.clone()))
    }
}

/// Case-insensitive multi-token & fuzzy subsequence matcher for database objects.
/// Every whitespace-separated token in `query` must match either as a substring
/// or an ordered subsequence of `name` or `schema.name`.
pub(crate) fn matches_search_query(schema: &str, name: &str, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let name_lower = name.to_lowercase();
    let full_lower = if schema.is_empty() {
        name_lower.clone()
    } else {
        format!("{}.{name_lower}", schema.to_lowercase())
    };
    q.split_whitespace().all(|raw_tok| {
        let tok = raw_tok.to_lowercase();
        name_lower.contains(&tok)
            || full_lower.contains(&tok)
            || is_subsequence(&tok, &full_lower)
    })
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut h_chars = haystack.chars();
    for nc in needle.chars() {
        if !h_chars.any(|hc| hc == nc) {
            return false;
        }
    }
    true
}

pub(crate) fn build_sidebar_tooltip(
    text: String,
    t: VeloTheme,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    Tooltip::new(text)
        .bg(t.panel_bg)
        .text_color(t.text_1)
        .border_color(t.border_strong)
        .text_size(t.fs_sm())
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(6.0))
        .whitespace_nowrap()
        .build(window, cx)
}

type ObjectGroups = BTreeMap<String, Vec<SchemaObject>>;

enum TreeGroupSource {
    /// References a group in `catalog` directly (optionally filtered by index).
    Catalog { filtered: Option<Vec<usize>> },
    /// Synthetic "收藏" group referencing `(catalog_group_name, index_in_group)`.
    Favorites(Vec<(String, usize)>),
}

struct TreeGroup {
    name: String,
    start: usize,
    count: usize,
    is_favorites: bool,
    source: TreeGroupSource,
}

enum TreeRow<'a> {
    Header(&'a TreeGroup),
    Object(&'a TreeGroup, usize, &'a SchemaObject),
}

pub(super) struct SidebarTree {
    catalog: Arc<ObjectGroups>,
    filter: String,
    collapsed: HashSet<String>,
    favorites: HashSet<(String, String)>,
    favorites_only: bool,
    groups: Vec<TreeGroup>,
    rows: usize,
    pub(super) total_objects: usize,
    pub(super) matched_objects: usize,
}

impl SidebarTree {
    fn new(
        catalog: Arc<ObjectGroups>,
        filter: &str,
        collapsed: &HashSet<String>,
        favorites: &HashSet<(String, String)>,
        favorites_only: bool,
    ) -> Self {
        let trimmed = filter.trim();
        let has_query = !trimmed.is_empty();
        let mut groups = Vec::with_capacity(catalog.len() + 1);
        let mut rows = 0;
        let mut total_objects = 0;
        let mut matched_objects = 0;

        // Collect favorited items in catalog order for the pinned "收藏" group.
        let mut fav_refs: Vec<(String, usize)> = Vec::new();
        for (gname, objects) in catalog.iter() {
            total_objects += objects.len();
            for (i, obj) in objects.iter().enumerate() {
                if favorites.contains(&(obj.schema.clone(), obj.name.clone()))
                    && matches_search_query(&obj.schema, &obj.name, trimmed)
                {
                    fav_refs.push((gname.clone(), i));
                }
            }
        }

        if !fav_refs.is_empty() {
            let count = fav_refs.len();
            groups.push(TreeGroup {
                name: FAVORITES_GROUP_NAME.to_string(),
                start: rows,
                count,
                is_favorites: true,
                source: TreeGroupSource::Favorites(fav_refs),
            });
            rows += 1;
            if has_query || favorites_only || !collapsed.contains(FAVORITES_GROUP_NAME) {
                rows += count;
            }
            if favorites_only {
                matched_objects = count;
            }
        }

        if !favorites_only {
            for (name, objects) in catalog.iter() {
                let filtered: Option<Vec<usize>> = has_query.then(|| {
                    objects
                        .iter()
                        .enumerate()
                        .filter_map(|(i, obj)| {
                            matches_search_query(&obj.schema, &obj.name, trimmed).then_some(i)
                        })
                        .collect()
                });
                let count = filtered.as_ref().map_or(objects.len(), Vec::len);
                matched_objects += count;
                if count == 0 {
                    continue;
                }
                groups.push(TreeGroup {
                    name: name.clone(),
                    start: rows,
                    count,
                    is_favorites: false,
                    source: TreeGroupSource::Catalog { filtered },
                });
                rows += 1;
                // Auto-expand groups when searching so matches are immediately visible.
                if has_query || !collapsed.contains(name) {
                    rows += count;
                }
            }
        }

        Self {
            catalog,
            filter: filter.to_owned(),
            collapsed: collapsed.clone(),
            favorites: favorites.clone(),
            favorites_only,
            groups,
            rows,
            total_objects,
            matched_objects,
        }
    }

    fn row(&self, index: usize) -> Option<TreeRow<'_>> {
        if index >= self.rows {
            return None;
        }
        let group = &self.groups[self.groups.partition_point(|group| group.start <= index) - 1];
        let offset = index - group.start;
        if offset == 0 {
            return Some(TreeRow::Header(group));
        }
        let offset = offset - 1;
        match &group.source {
            TreeGroupSource::Catalog { filtered } => {
                let item = filtered.as_ref().map_or(offset, |indices| indices[offset]);
                Some(TreeRow::Object(group, item, &self.catalog[&group.name][item]))
            }
            TreeGroupSource::Favorites(refs) => {
                let (ref gname, item) = refs[offset];
                Some(TreeRow::Object(group, offset, &self.catalog[gname][item]))
            }
        }
    }

    fn first_openable(&self) -> Option<(String, String)> {
        for i in 0..self.rows {
            if let Some(TreeRow::Object(_, _, obj)) = self.row(i) {
                if matches!(
                    obj.object_type,
                    SchemaObjectType::Table
                        | SchemaObjectType::View
                        | SchemaObjectType::MaterializedView
                ) {
                    return Some((obj.schema.clone(), obj.name.clone()));
                }
            }
        }
        None
    }
}

impl VeloDbApp {
    pub(crate) fn first_matching_openable_object(&self) -> Option<(String, String)> {
        if let Some(tree) = self.sidebar_tree.as_ref() {
            return tree.first_openable();
        }
        let favs = self.current_schema_favorites();
        let tree = SidebarTree::new(
            Arc::clone(&self.data.schema_objects),
            &self.data.sidebar_filter,
            &self.data.collapsed_groups,
            &favs,
            self.data.sidebar_favorites_only,
        );
        tree.first_openable()
    }

    fn render_tree_group(
        &self,
        group: &TreeGroup,
        t: &VeloTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let gname = group.name.clone();
        let is_collapsed = self.data.sidebar_filter.trim().is_empty()
            && !(self.data.sidebar_favorites_only && group.is_favorites)
            && self.data.collapsed_groups.contains(&gname);
        let chevron = if is_collapsed { "arrow-right" } else { "arrow-down" };
        div()
            .id(gpui::SharedString::from(format!("sb-grp-{gname}")))
            .w_full()
            .px(px(8.0))
            .h(t.control_h())
            .rounded(px(4.0))
            .flex()
            .items_center()
            .justify_between()
            .text_size(t.fs_xs())
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(if group.is_favorites { t.amber } else { t.text_3 })
            .cursor_pointer()
            .hover(|s| s.bg(t.surface_1).text_color(if group.is_favorites { t.amber } else { t.text_2 }))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.data.collapsed_groups.contains(&gname) {
                    this.data.collapsed_groups.remove(&gname);
                } else {
                    this.data.collapsed_groups.insert(gname.clone());
                }
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .child(hgi(
                        chevron,
                        10.0,
                        if group.is_favorites { t.amber } else { t.text_3 },
                    ))
                    .when(group.is_favorites, |d| {
                        d.child(hgi("star-filled", 11.0, t.amber))
                    })
                    .child(group.name.clone()),
            )
            .child(group.count.to_string())
            .into_any_element()
    }

    fn render_tree_object(
        &self,
        group_name: &str,
        idx: usize,
        obj: &SchemaObject,
        t: &VeloTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_active = self
            .active_tab()
            .and_then(|tab| tab.table_state.as_ref())
            .is_some_and(|ts| ts.schema == obj.schema && ts.table == obj.name);
        let is_fav = self.is_favorite(&obj.schema, &obj.name);
        let obj_type = obj.object_type;
        let icon = match obj_type {
            SchemaObjectType::Table => "table",
            SchemaObjectType::View | SchemaObjectType::MaterializedView => "view",
            SchemaObjectType::Function => "func",
            SchemaObjectType::Sequence => "columns",
            SchemaObjectType::Enum => "tag",
            SchemaObjectType::Extension => "puzzle",
        };
        let icon_color = if is_active { t.accent } else { t.text_2 };
        let schema = gpui::SharedString::from(obj.schema.clone());
        let name = gpui::SharedString::from(obj.name.clone());
        let display_name = name.clone();
        let r_schema = schema.clone();
        let r_name = name.clone();
        let fav_schema = schema.clone();
        let fav_name = name.clone();
        let tooltip_name = name.clone();
        let tooltip_theme = *t;
        let star_tip_theme = *t;
        let can_open = matches!(
            obj_type,
            SchemaObjectType::Table | SchemaObjectType::View | SchemaObjectType::MaterializedView
        );
        div()
            .id(gpui::SharedString::from(format!(
                "sb-obj-{group_name}-{}-{idx}",
                obj.name
            )))
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(8.0))
            .h(t.control_h())
            .rounded(px(6.0))
            .cursor_pointer()
            .bg(if is_active {
                t.accent_soft
            } else {
                gpui::transparent_black()
            })
            .text_color(if is_active { t.accent } else { t.text_2 })
            .font_weight(if is_active {
                gpui::FontWeight::MEDIUM
            } else {
                gpui::FontWeight::NORMAL
            })
            .hover(|s| s.bg(t.surface_2).text_color(t.text_1))
            .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
            .tooltip(move |window, cx| {
                build_sidebar_tooltip(tooltip_name.to_string(), tooltip_theme, window, cx)
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if can_open {
                    this.open_table_tab(&schema, &name, None, window, cx);
                } else {
                    this.show_toast(format!("对象: {schema}.{name}"), cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.data.ctx_menu.open = false;
                    this.data.tab_ctx_menu.open = false;
                    this.data.sidebar_ctx_menu.open = true;
                    this.data.sidebar_ctx_menu.x = f32::from(ev.position.x);
                    this.data.sidebar_ctx_menu.y = f32::from(ev.position.y);
                    this.data.sidebar_ctx_menu.schema = r_schema.to_string();
                    this.data.sidebar_ctx_menu.name = r_name.to_string();
                    this.data.sidebar_ctx_menu.object_type = obj_type;
                    this.data.sidebar_ctx_menu.can_open = can_open;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .opacity(0.75)
                    .child(hgi(icon, 14.0, icon_color)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(t.fs_md())
                    .child(display_name),
            )
            .child(
                div()
                    .id(gpui::SharedString::from(format!(
                        "sb-fav-{group_name}-{}-{idx}",
                        obj.name
                    )))
                    .flex_shrink_0()
                    .w(px(20.0))
                    .h(px(20.0))
                    .rounded(px(4.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(if is_fav { 0.95 } else { 0.35 })
                    .hover(|s| s.opacity(1.0).bg(t.surface_3))
                    .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                    .tooltip(move |window, cx| {
                        build_sidebar_tooltip(
                            if is_fav {
                                "取消收藏".to_string()
                            } else {
                                "收藏".to_string()
                            },
                            star_tip_theme,
                            window,
                            cx,
                        )
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_favorite(&fav_schema, &fav_name, cx);
                    }))
                    .child(hgi(
                        if is_fav { "star-filled" } else { "star" },
                        12.0,
                        if is_fav { t.amber } else { t.text_3 },
                    )),
            )
            .into_any_element()
    }

    fn ensure_sidebar_tree(&mut self) -> Rc<SidebarTree> {
        let catalog = &self.data.schema_objects;
        let filter = &self.data.sidebar_filter;
        let collapsed = &self.data.collapsed_groups;
        let favorites_only = self.data.sidebar_favorites_only;
        let current_favs = self.current_schema_favorites();

        let reset = self.sidebar_tree.as_ref().is_none_or(|tree| {
            !Arc::ptr_eq(&tree.catalog, catalog)
                || tree.filter != *filter
                || tree.favorites_only != favorites_only
        });
        let needs_rebuild = reset
            || self.sidebar_tree.as_ref().is_some_and(|tree| {
                tree.collapsed != *collapsed || tree.favorites != current_favs
            });
        if needs_rebuild {
            self.sidebar_tree = Some(Rc::new(SidebarTree::new(
                Arc::clone(catalog),
                filter,
                collapsed,
                &current_favs,
                favorites_only,
            )));
            if reset {
                self.sidebar_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            }
        }
        Rc::clone(self.sidebar_tree.as_ref().unwrap())
    }

    fn render_object_tree(&mut self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let tree = self.ensure_sidebar_tree();
        if tree.rows == 0 {
            let empty_msg = if self.data.sidebar_favorites_only
                && self.data.sidebar_filter.trim().is_empty()
            {
                "当前暂无收藏的表或对象"
            } else {
                "没有匹配的数据库对象"
            };
            return div()
                .p(px(16.0))
                .text_center()
                .text_size(t.fs_sm())
                .text_color(t.text_3)
                .child(empty_msg)
                .into_any_element();
        }
        let theme = *t;
        let list = uniform_list(
            "schema-tree",
            tree.rows,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .filter_map(|index| {
                        Some(match tree.row(index)? {
                            TreeRow::Header(group) => this.render_tree_group(group, &theme, cx),
                            TreeRow::Object(group, idx, obj) => {
                                this.render_tree_object(&group.name, idx, obj, &theme, cx)
                            }
                        })
                    })
                    .collect()
            }),
        )
        .size_full()
        .track_scroll(&self.sidebar_scroll);
        div()
            .size_full()
            .relative()
            .child(list)
            .child(
                div().absolute().inset_0().child(
                    Scrollbar::vertical(&self.sidebar_scroll)
                        .id("schema-tree-scrollbar")
                        .viewport_from_layout(),
                ),
            )
            .into_any_element()
    }

    pub fn render_sidebar(&mut self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let is_mysql = self
            .data
            .active_conn
            .as_ref()
            .map(|c| c.driver == crate::adapter::DbDriver::Mysql)
            .unwrap_or(false);
        let has_selected_db = !self.data.current_db.trim().is_empty();

        let sidebar_w = self.data.settings.sidebar_width.clamp(180.0, 520.0);
        let resizing = self.data.sidebar_resizing;

        div()
            .w(px(sidebar_w))
            .flex_shrink_0()
            .bg(t.sidebar_bg)
            .border_r_1()
            .border_color(t.border)
            .flex()
            .flex_col()
            .min_h_0()
            .relative()
            // Sidebar top: database switcher + (PG schema switcher) + refresh + filter
            .child(
                div()
                    .p(px(10.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .id("db-dropdown-btn")
                                    .flex_1()
                                    .min_w_0()
                                    .px(px(8.0))
                                    .py(px(6.0))
                                    .rounded(px(6.0))
                                    .bg(t.surface_2)
                                    .border_1()
                                    .border_color(if self.data.db_dropdown_open {
                                        t.accent
                                    } else {
                                        t.border
                                    })
                                    .text_size(t.fs_md())
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(if has_selected_db { t.text_1 } else { t.text_3 })
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.surface_3))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(4.0))
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation();
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.data.db_dropdown_open = !this.data.db_dropdown_open;
                                        this.data.schema_dropdown_open = false;
                                        cx.notify();
                                    }))
                                    .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                                    .tooltip({
                                        let db_tip = if has_selected_db {
                                            self.data.current_db.clone()
                                        } else {
                                            "请选择数据库".to_string()
                                        };
                                        let tip_theme = *t;
                                        move |window, cx| {
                                            build_sidebar_tooltip(db_tip.clone(), tip_theme, window, cx)
                                        }
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .items_center()
                                            .gap(px(5.0))
                                            .child(
                                                div()
                                                    .flex_shrink_0()
                                                    .child(hgi("database", 12.0, t.text_3)),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .child(if has_selected_db {
                                                        format!("db: {}", self.data.current_db)
                                                    } else {
                                                        "请选择数据库".to_string()
                                                    }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .child(hgi("arrow-down", 12.0, t.text_3)),
                                    ),
                            )
                            .child(
                                div()
                                    .id("refresh-schema-btn")
                                    .h(px(28.0))
                                    .w(px(28.0))
                                    .rounded(px(6.0))
                                    .bg(t.surface_2)
                                    .border_1()
                                    .border_color(t.border)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.surface_3))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let s = this.data.current_schema.clone();
                                        this.refresh_schema(s, cx);
                                    }))
                                    .child(hgi("refresh", 14.0, t.text_2)),
                            ),
                    )
                    .when(!is_mysql && has_selected_db, |d| {
                        d.child(
                            div()
                                .id("schema-dropdown-btn")
                                .w_full()
                                .px(px(8.0))
                                .py(px(5.0))
                                .rounded(px(6.0))
                                .bg(t.surface_1)
                                .border_1()
                                .border_color(if self.data.schema_dropdown_open {
                                    t.accent
                                } else {
                                    t.border
                                })
                                .text_size(t.fs_sm())
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(t.text_2)
                                .cursor_pointer()
                                .hover(|s| s.bg(t.surface_2).text_color(t.text_1))
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(4.0))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.data.schema_dropdown_open =
                                        !this.data.schema_dropdown_open;
                                    this.data.db_dropdown_open = false;
                                    cx.notify();
                                }))
                                .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                                .tooltip({
                                    let sch_tip = self.data.current_schema.clone();
                                    let tip_theme = *t;
                                    move |window, cx| {
                                        build_sidebar_tooltip(sch_tip.clone(), tip_theme, window, cx)
                                    }
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(format!("schema: {}", self.data.current_schema)),
                                )
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .child(hgi("arrow-down", 12.0, t.text_3)),
                                ),
                        )
                    })
                    .child({
                        let fav_only = self.data.sidebar_favorites_only;
                        let is_filtering =
                            !self.data.sidebar_filter.trim().is_empty() || fav_only;
                        let match_badge = if has_selected_db
                            && !self.data.schema_objects.is_empty()
                            && is_filtering
                        {
                            let tree = self.ensure_sidebar_tree();
                            Some(format!("{}/{}", tree.matched_objects, tree.total_objects))
                        } else {
                            None
                        };
                        let fav_tip_theme = *t;
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h(t.control_h())
                                    .rounded(px(6.0))
                                    .bg(t.surface_1)
                                    .border_1()
                                    .border_color(if !self.data.sidebar_filter.trim().is_empty() {
                                        t.accent
                                    } else {
                                        t.border
                                    })
                                    .px(px(8.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .child(hgi("search", 13.0, t.text_3))
                                    .child(
                                        div().flex_1().min_w_0().h_full().flex().items_center().child(
                                            Input::new(&self.sidebar_filter_input)
                                                .appearance(false)
                                                .cleanable(true)
                                                .px_0()
                                                .py_0()
                                                .h_full()
                                                .text_size(t.fs_md()),
                                        ),
                                    )
                                    .when_some(match_badge, |d, badge| {
                                        d.child(
                                            div()
                                                .flex_shrink_0()
                                                .text_size(t.fs_xs())
                                                .text_color(t.text_3)
                                                .child(badge),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .id("sidebar-fav-filter-btn")
                                    .flex_shrink_0()
                                    .h(t.control_h())
                                    .w(t.control_h())
                                    .rounded(px(6.0))
                                    .bg(if fav_only { t.amber_soft } else { t.surface_2 })
                                    .border_1()
                                    .border_color(if fav_only { t.amber } else { t.border })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| {
                                        s.bg(if fav_only {
                                            t.amber_soft
                                        } else {
                                            t.surface_3
                                        })
                                    })
                                    .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                                    .tooltip(move |window, cx| {
                                        build_sidebar_tooltip(
                                            if fav_only {
                                                "显示全部对象".to_string()
                                            } else {
                                                "仅显示收藏".to_string()
                                            },
                                            fav_tip_theme,
                                            window,
                                            cx,
                                        )
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.data.sidebar_favorites_only =
                                            !this.data.sidebar_favorites_only;
                                        this.sidebar_tree = None;
                                        cx.notify();
                                    }))
                                    .child(hgi(
                                        if fav_only { "star-filled" } else { "star" },
                                        13.0,
                                        if fav_only { t.amber } else { t.text_2 },
                                    )),
                            )
                    }),
            )
            // Tree
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .p(px(6.0))
                    .when(has_selected_db && !self.data.schema_objects.is_empty(), |d| {
                        d.child(self.render_object_tree(t, cx))
                    })
                    .when(!has_selected_db, |d| {
                        d.child(
                            div()
                                .px(px(16.0))
                                .py(px(36.0))
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap(px(10.0))
                                .child(
                                    div()
                                        .w(px(36.0))
                                        .h(px(36.0))
                                        .rounded(px(9.0))
                                        .bg(t.surface_2)
                                        .border_1()
                                        .border_color(t.border)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(hgi("database", 16.0, t.text_3)),
                                )
                                .child(
                                    div()
                                        .text_size(t.fs_md())
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(t.text_2)
                                        .child("尚未选择数据库"),
                                )
                                .child(
                                    div()
                                        .text_size(t.fs_sm())
                                        .text_color(t.text_3)
                                        .text_center()
                                        .child("请先选择一个数据库以浏览数据表与对象"),
                                )
                                .when(!self.data.databases.is_empty(), |d| {
                                    d.child(
                                        div()
                                            .id("sidebar-pick-db-btn")
                                            .mt(px(4.0))
                                            .h(t.control_h())
                                            .px(px(12.0))
                                            .rounded(px(6.0))
                                            .bg(t.accent_soft)
                                            .border_1()
                                            .border_color(t.accent)
                                            .text_size(t.fs_md())
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(t.accent)
                                            .flex()
                                            .items_center()
                                            .gap(px(5.0))
                                            .cursor_pointer()
                                            .hover(|s| s.opacity(0.9))
                                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                cx.stop_propagation();
                                            })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.data.db_dropdown_open = true;
                                                this.data.schema_dropdown_open = false;
                                                cx.notify();
                                            }))
                                            .child("选择数据库"),
                                    )
                                }),
                        )
                    })
                    .when(has_selected_db && self.data.schema_objects.is_empty(), |d| {
                        d.child(
                            div()
                                .p(px(16.0))
                                .text_center()
                                .text_size(t.fs_sm())
                                .text_color(t.text_3)
                                .child("当前数据库暂无对象"),
                        )
                    }),
            )
            // Database Dropdown Popover (rendered on top via deferred + absolute outer container)
            .when(self.data.db_dropdown_open, |d| {
                d.child(
                    deferred(
                        div()
                            .id("db-dropdown-overlay")
                            .absolute()
                            .top(px(42.0))
                            .left(px(10.0))
                            .right(px(10.0))
                            .occlude()
                            .rounded(px(8.0))
                            .bg(t.panel_bg)
                            .border_1()
                            .border_color(t.border_strong)
                            .shadow_lg()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .child(
                                div()
                                    .id("db-dropdown-scroll")
                                    .w_full()
                                    .min_w_0()
                                    .max_h(px(280.0))
                                    .overflow_y_scrollbar()
                                    .p(px(4.0))
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .when(self.data.databases.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .px(px(8.0))
                                                .py(px(10.0))
                                                .text_center()
                                                .text_size(t.fs_sm())
                                                .text_color(t.text_3)
                                                .child("暂无可选数据库"),
                                        )
                                    })
                                    .children(self.data.databases.iter().enumerate().map(
                                        |(i, db_name)| {
                                            let is_cur = db_name == &self.data.current_db;
                                            let target_db = db_name.clone();
                                            div()
                                                .id(("db-opt", i))
                                                .w_full()
                                                .min_w_0()
                                                .px(px(8.0))
                                                .py(px(6.0))
                                                .rounded(px(5.0))
                                                .flex()
                                                .items_center()
                                                .justify_between()
                                                .gap(px(6.0))
                                                .text_size(t.fs_md())
                                                .cursor_pointer()
                                                .bg(if is_cur {
                                                    t.accent_soft
                                                } else {
                                                    gpui::transparent_black()
                                                })
                                                .text_color(if is_cur {
                                                    t.accent
                                                } else {
                                                    t.text_1
                                                })
                                                .font_weight(if is_cur {
                                                    gpui::FontWeight::SEMIBOLD
                                                } else {
                                                    gpui::FontWeight::NORMAL
                                                })
                                                .hover(|s| s.bg(t.surface_2))
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(move |this, _, _, cx| {
                                                        cx.stop_propagation();
                                                        this.switch_database(
                                                            target_db.clone(),
                                                            cx,
                                                        );
                                                    }),
                                                )
                                                .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                                                .tooltip({
                                                    let db_tip = db_name.clone();
                                                    let tip_theme = *t;
                                                    move |window, cx| {
                                                        build_sidebar_tooltip(
                                                            db_tip.clone(),
                                                            tip_theme,
                                                            window,
                                                            cx,
                                                        )
                                                    }
                                                })
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .flex()
                                                        .items_center()
                                                        .gap(px(6.0))
                                                        .child(
                                                            div()
                                                                .flex_shrink_0()
                                                                .child(hgi(
                                                                    "database",
                                                                    12.0,
                                                                    if is_cur {
                                                                        t.accent
                                                                    } else {
                                                                        t.text_3
                                                                    },
                                                                )),
                                                        )
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .min_w_0()
                                                                .truncate()
                                                                .child(db_name.clone()),
                                                        ),
                                                )
                                                .when(is_cur, |d| {
                                                    d.child(
                                                        div()
                                                            .text_size(t.fs_xs())
                                                            .flex_shrink_0()
                                                            .child("当前"),
                                                    )
                                                })
                                        },
                                    )),
                            ),
                    )
                    .with_priority(10),
                )
            })
            // Schema Dropdown Popover (PostgreSQL)
            .when(self.data.schema_dropdown_open && !is_mysql, |d| {
                d.child(
                    deferred(
                        div()
                            .id("schema-dropdown-overlay")
                            .absolute()
                            .top(px(76.0))
                            .left(px(10.0))
                            .right(px(10.0))
                            .occlude()
                            .rounded(px(8.0))
                            .bg(t.panel_bg)
                            .border_1()
                            .border_color(t.border_strong)
                            .shadow_lg()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .child(
                                div()
                                    .id("schema-dropdown-scroll")
                                    .w_full()
                                    .min_w_0()
                                    .max_h(px(240.0))
                                    .overflow_y_scrollbar()
                                    .p(px(4.0))
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .children(self.data.schemas.iter().enumerate().map(
                                        |(i, sch_name)| {
                                            let is_cur = sch_name == &self.data.current_schema;
                                            let target_sch = sch_name.clone();
                                            div()
                                                .id(("sch-opt", i))
                                                .w_full()
                                                .min_w_0()
                                                .px(px(8.0))
                                                .py(px(6.0))
                                                .rounded(px(5.0))
                                                .flex()
                                                .items_center()
                                                .justify_between()
                                                .gap(px(6.0))
                                                .text_size(t.fs_md())
                                                .cursor_pointer()
                                                .bg(if is_cur {
                                                    t.accent_soft
                                                } else {
                                                    gpui::transparent_black()
                                                })
                                                .text_color(if is_cur {
                                                    t.accent
                                                } else {
                                                    t.text_1
                                                })
                                                .font_weight(if is_cur {
                                                    gpui::FontWeight::SEMIBOLD
                                                } else {
                                                    gpui::FontWeight::NORMAL
                                                })
                                                .hover(|s| s.bg(t.surface_2))
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(move |this, _, _, cx| {
                                                        cx.stop_propagation();
                                                        this.refresh_schema(
                                                            target_sch.clone(),
                                                            cx,
                                                        );
                                                    }),
                                                )
                                                .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                                                .tooltip({
                                                    let sch_tip = sch_name.clone();
                                                    let tip_theme = *t;
                                                    move |window, cx| {
                                                        build_sidebar_tooltip(
                                                            sch_tip.clone(),
                                                            tip_theme,
                                                            window,
                                                            cx,
                                                        )
                                                    }
                                                })
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .truncate()
                                                        .child(sch_name.clone()),
                                                )
                                                .when(is_cur, |d| {
                                                    d.child(
                                                        div()
                                                            .text_size(t.fs_xs())
                                                            .flex_shrink_0()
                                                            .child("当前"),
                                                    )
                                                })
                                        },
                                    )),
                            ),
                    )
                    .with_priority(10),
                )
            })
            // Right-edge draggable resize handle
            .child(
                div()
                    .id("sidebar-resize-handle")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(-3.0))
                    .w(px(6.0))
                    .cursor_col_resize()
                    .flex()
                    .justify_center()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.data.sidebar_resizing = true;
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

    pub fn render_tabbar(&self, t: &VeloTheme, cx: &mut Context<Self>) -> AnyElement {
        let tab_count = self.data.tabs.len();
        let can_drag = tab_count > 1;
        let tabs_els: Vec<AnyElement> = self
            .data
            .tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| {
                let active = tab.id == self.data.active_tab_id;
                let tab_id = tab.id.clone();
                let mid_close_id = tab.id.clone();
                let ctx_tab_id = tab.id.clone();
                let close_id = tab.id.clone();
                let drag_over_tab_id = tab.id.clone();
                let tip_title = tab.title.clone();
                let tip_theme = *t;
                let ico = match tab.kind {
                    TabKind::Table => "table",
                    TabKind::Monitor => "activity",
                    TabKind::Sql => "code",
                };
                let ico_color = if active { t.text_1 } else { t.text_2 };
                let drag_payload = DraggedTab {
                    tab_id: tab.id.clone(),
                    from_idx: idx,
                    title: tab.title.clone(),
                    icon: ico,
                    theme: *t,
                };
                let accent = t.accent;
                let hover_bg = t.surface_2;
                let item_id = gpui::SharedString::from(format!("tab-item-{}", tab.id));
                let close_btn_id = gpui::SharedString::from(format!("close-tab-{}", tab.id));

                div()
                    .id(item_id)
                    .h(t.control_h())
                    .max_w(px(200.0))
                    .flex_shrink_0()
                    .px(px(10.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .text_size(t.fs_md())
                    .cursor_pointer()
                    .bg(if active {
                        t.panel_bg
                    } else {
                        gpui::transparent_black()
                    })
                    .border_1()
                    .border_color(if active {
                        t.border
                    } else {
                        gpui::transparent_black()
                    })
                    .text_color(if active { t.text_1 } else { t.text_2 })
                    .font_weight(if active {
                        gpui::FontWeight::MEDIUM
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .hover(|s| s.bg(t.surface_2).text_color(t.text_1))
                    .tooltip_show_delay(SIDEBAR_TOOLTIP_DELAY)
                    .tooltip(move |window, cx| {
                        build_sidebar_tooltip(tip_title.clone(), tip_theme, window, cx)
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_tab(tab_id.clone(), window, cx);
                    }))
                    .when(can_drag, |d| {
                        d.on_drag(drag_payload, |drag, _, _, cx| {
                            cx.stop_propagation();
                            cx.new(|_| DraggedTabView {
                                title: drag.title.clone(),
                                icon: drag.icon,
                                theme: drag.theme,
                            })
                        })
                        .drag_over::<DraggedTab>(move |style, drag, _, _| {
                            if drag.tab_id == drag_over_tab_id {
                                return style;
                            }
                            if drag.from_idx < idx {
                                style
                                    .rounded_r(px(0.0))
                                    .border_t_0()
                                    .border_b_0()
                                    .border_l_0()
                                    .border_r_2()
                                    .border_color(accent)
                                    .bg(hover_bg)
                            } else {
                                style
                                    .rounded_l(px(0.0))
                                    .border_t_0()
                                    .border_b_0()
                                    .border_r_0()
                                    .border_l_2()
                                    .border_color(accent)
                                    .bg(hover_bg)
                            }
                        })
                        .on_drop(cx.listener(move |this, drag: &DraggedTab, _, cx| {
                            this.reorder_tab(&drag.tab_id, idx, cx);
                        }))
                    })
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_tab(&mid_close_id, window, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.data.ctx_menu.open = false;
                            this.data.sidebar_ctx_menu.open = false;
                            this.data.tab_ctx_menu.open = true;
                            this.data.tab_ctx_menu.x = f32::from(ev.position.x);
                            this.data.tab_ctx_menu.y = f32::from(ev.position.y);
                            this.data.tab_ctx_menu.tab_id = ctx_tab_id.clone();
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .opacity(0.8)
                            .child(hgi(ico, 13.0, ico_color)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(tab.title.clone()),
                    )
                    .child(
                        div()
                            .id(close_btn_id)
                            .flex_shrink_0()
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded(px(4.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(t.surface_3))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_tab(&close_id, window, cx);
                            }))
                            .child(hgi("cancel", 10.0, t.text_3)),
                    )
                    .into_any_element()
            })
            .collect();

        let empty_accent = t.accent;
        div()
            .h(px(38.0))
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .border_b_1()
            .border_color(t.border)
            .bg(t.surface_1)
            .child(
                div()
                    .id("tabbar-scroll")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_x_scroll()
                    .lock_scroll_axis()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .children(tabs_els)
                    .when(can_drag, |d| {
                        d.child(
                            div()
                                .id("tab-bar-empty-space")
                                .h(t.control_h())
                                .flex_1()
                                .min_w(px(24.0))
                                .drag_over::<DraggedTab>(move |style, drag, _, _| {
                                    if drag.from_idx + 1 < tab_count {
                                        style.border_l_2().border_color(empty_accent)
                                    } else {
                                        style
                                    }
                                })
                                .on_drop(cx.listener(move |this, drag: &DraggedTab, _, cx| {
                                    let last = this.data.tabs.len().saturating_sub(1);
                                    this.reorder_tab(&drag.tab_id, last, cx);
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    pub fn render_sidebar_context_menu(
        &self,
        t: &VeloTheme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cm = &self.data.sidebar_ctx_menu;
        let schema = cm.schema.clone();
        let name = cm.name.clone();
        let can_open = cm.can_open;
        let is_table = cm.object_type == SchemaObjectType::Table;
        let is_fav = self.is_favorite(&schema, &name);
        let read_only = self.data.active_conn.as_ref().is_some_and(|c| c.read_only);

        let est_height = if is_table {
            430.0
        } else if can_open {
            330.0
        } else {
            175.0
        };
        let vp = window.viewport_size();
        let max_x = (f32::from(vp.width) - 236.0).max(8.0);
        let max_y = (f32::from(vp.height) - est_height).max(8.0);

        let menu_item = |id: &'static str,
                         icon: &'static str,
                         icon_color: gpui::Hsla,
                         label: &'static str,
                         danger: bool,
                         disabled: bool,
                         on_click: Box<dyn Fn(&gpui::ClickEvent, &mut Window, &mut App)>| {
            div()
                .id(id)
                .px(px(9.0))
                .py(px(6.0))
                .rounded(px(5.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(t.fs_md())
                .text_color(if disabled {
                    t.text_3
                } else if danger {
                    t.red
                } else {
                    t.text_1
                })
                .when(!disabled, |d| {
                    d.cursor_pointer()
                        .hover(move |s| {
                            s.bg(if danger { t.red } else { t.accent })
                                .text_color(gpui::white())
                        })
                        .on_click(on_click)
                })
                .when(disabled, |d| d.opacity(0.55))
                .child(hgi(icon, 13.0, if disabled { t.text_3 } else { icon_color }))
                .child(label)
        };

        let s_open = schema.clone();
        let n_open = name.clone();
        let s_struct = schema.clone();
        let n_struct = name.clone();
        let s_ddl = schema.clone();
        let n_ddl = name.clone();
        let s_fav = schema.clone();
        let n_fav = name.clone();
        let s_sel = schema.clone();
        let n_sel = name.clone();
        let s_cnt = schema.clone();
        let n_cnt = name.clone();
        let n_copy = name.clone();
        let s_copy_ddl = schema.clone();
        let n_copy_ddl = name.clone();
        let s_exp = schema.clone();
        let n_exp = name.clone();
        let s_ren = schema.clone();
        let n_ren = name.clone();
        let s_trunc = schema.clone();
        let n_trunc = name.clone();
        let s_drop = schema.clone();
        let n_drop = name.clone();
        let full_name = if schema.is_empty() {
            name.clone()
        } else {
            format!("{schema}.{name}")
        };

        div()
            .id("sidebar-ctx-menu")
            .absolute()
            .left(px(cm.x.clamp(8.0, max_x)))
            .top(px(cm.y.clamp(8.0, max_y)))
            .w(px(220.0))
            .p(px(5.0))
            .rounded(px(9.0))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border_strong)
            .shadow_lg()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .when(can_open, |d| {
                d.child(menu_item(
                    "sb-ctx-open-data",
                    "table",
                    t.text_2,
                    "打开表数据",
                    false,
                    false,
                    Box::new(cx.listener(move |this, _, window, cx| {
                        this.data.sidebar_ctx_menu.open = false;
                        this.open_table_tab_with_subview(
                            &s_open,
                            &n_open,
                            None,
                            TableSubview::Data,
                            window,
                            cx,
                        );
                    })),
                ))
                .child(menu_item(
                    "sb-ctx-open-struct",
                    "columns",
                    t.text_2,
                    "查看表结构",
                    false,
                    false,
                    Box::new(cx.listener(move |this, _, window, cx| {
                        this.data.sidebar_ctx_menu.open = false;
                        this.open_table_tab_with_subview(
                            &s_struct,
                            &n_struct,
                            None,
                            TableSubview::Structure,
                            window,
                            cx,
                        );
                    })),
                ))
                .child(menu_item(
                    "sb-ctx-open-ddl",
                    "file-code",
                    t.text_2,
                    "查看建表语句 (DDL)",
                    false,
                    false,
                    Box::new(cx.listener(move |this, _, window, cx| {
                        this.data.sidebar_ctx_menu.open = false;
                        this.open_table_ddl_tab(&s_ddl, &n_ddl, window, cx);
                    })),
                ))
            })
            .child(menu_item(
                "sb-ctx-toggle-fav",
                if is_fav { "star-filled" } else { "star" },
                if is_fav { t.amber } else { t.text_2 },
                if is_fav { "取消收藏" } else { "收藏" },
                false,
                false,
                Box::new(cx.listener(move |this, _, _, cx| {
                    this.data.sidebar_ctx_menu.open = false;
                    this.toggle_favorite(&s_fav, &n_fav, cx);
                })),
            ))
            .when(can_open, |d| {
                d.child(div().h(px(1.0)).bg(t.border).my(px(4.0)))
                    .child(menu_item(
                        "sb-ctx-select-sql",
                        "play",
                        t.text_2,
                        "新建 SELECT 查询",
                        false,
                        false,
                        Box::new(cx.listener(move |this, _, window, cx| {
                            this.data.sidebar_ctx_menu.open = false;
                            this.open_table_select_sql(&s_sel, &n_sel, window, cx);
                        })),
                    ))
                    .child(menu_item(
                        "sb-ctx-count-sql",
                        "activity",
                        t.text_2,
                        "统计总行数 (COUNT)",
                        false,
                        false,
                        Box::new(cx.listener(move |this, _, window, cx| {
                            this.data.sidebar_ctx_menu.open = false;
                            this.open_table_count_sql(&s_cnt, &n_cnt, window, cx);
                        })),
                    ))
            })
            .child(div().h(px(1.0)).bg(t.border).my(px(4.0)))
            .child(menu_item(
                "sb-ctx-copy-name",
                "copy",
                t.text_2,
                "复制名称",
                false,
                false,
                Box::new(cx.listener(move |this, _, _, cx| {
                    this.data.sidebar_ctx_menu.open = false;
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(n_copy.clone()));
                    this.show_toast(format!("已复制: {n_copy}"), cx);
                })),
            ))
            .child(menu_item(
                "sb-ctx-copy-full",
                "copy",
                t.text_2,
                "复制完整名 (schema.table)",
                false,
                false,
                Box::new(cx.listener(move |this, _, _, cx| {
                    this.data.sidebar_ctx_menu.open = false;
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(full_name.clone()));
                    this.show_toast(format!("已复制: {full_name}"), cx);
                })),
            ))
            .when(can_open, |d| {
                d.child(menu_item(
                    "sb-ctx-copy-ddl",
                    "code",
                    t.text_2,
                    "复制 DDL",
                    false,
                    false,
                    Box::new(cx.listener(move |this, _, _, cx| {
                        this.data.sidebar_ctx_menu.open = false;
                        this.copy_table_ddl(&s_copy_ddl, &n_copy_ddl, cx);
                    })),
                ))
                .child(menu_item(
                    "sb-ctx-export",
                    "download",
                    t.text_2,
                    "导出表数据...",
                    false,
                    false,
                    Box::new(cx.listener(move |this, _, window, cx| {
                        this.data.sidebar_ctx_menu.open = false;
                        this.open_table_export(&s_exp, &n_exp, window, cx);
                    })),
                ))
            })
            .when(is_table, |d| {
                d.child(div().h(px(1.0)).bg(t.border).my(px(4.0)))
                    .child(menu_item(
                        "sb-ctx-rename",
                        "edit",
                        t.text_2,
                        "重命名表...",
                        false,
                        read_only,
                        Box::new(cx.listener(move |this, _, window, cx| {
                            this.data.sidebar_ctx_menu.open = false;
                            this.open_rename_table_modal(&s_ren, &n_ren, window, cx);
                        })),
                    ))
                    .child(menu_item(
                        "sb-ctx-truncate",
                        "alert",
                        t.red,
                        "清空表 (TRUNCATE)...",
                        true,
                        read_only,
                        Box::new(cx.listener(move |this, _, _, cx| {
                            this.data.sidebar_ctx_menu.open = false;
                            this.open_table_danger_modal(
                                TableDangerAction::Truncate,
                                &s_trunc,
                                &n_trunc,
                                cx,
                            );
                        })),
                    ))
                    .child(menu_item(
                        "sb-ctx-drop",
                        "trash",
                        t.red,
                        "删除表 (DROP TABLE)...",
                        true,
                        read_only,
                        Box::new(cx.listener(move |this, _, _, cx| {
                            this.data.sidebar_ctx_menu.open = false;
                            this.open_table_danger_modal(
                                TableDangerAction::Drop,
                                &s_drop,
                                &n_drop,
                                cx,
                            );
                        })),
                    ))
            })
            .child(div().h(px(1.0)).bg(t.border).my(px(4.0)))
            .child(menu_item(
                "sb-ctx-refresh",
                "refresh",
                t.text_2,
                "刷新对象列表",
                false,
                false,
                Box::new(cx.listener(|this, _, _, cx| {
                    this.data.sidebar_ctx_menu.open = false;
                    let s = this.data.current_schema.clone();
                    this.refresh_schema(s, cx);
                })),
            ))
            .into_any_element()
    }

    pub fn render_tab_context_menu(
        &self,
        t: &VeloTheme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cm = &self.data.tab_ctx_menu;
        let tid1 = cm.tab_id.clone();
        let tid2 = cm.tab_id.clone();
        let tid3 = cm.tab_id.clone();
        let vp = window.viewport_size();
        let max_x = (f32::from(vp.width) - 186.0).max(8.0);
        let max_y = (f32::from(vp.height) - 160.0).max(8.0);

        let menu_item = |id: &'static str,
                         label: &'static str,
                         danger: bool,
                         on_click: Box<dyn Fn(&gpui::ClickEvent, &mut Window, &mut App)>| {
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
            .id("tab-ctx-menu")
            .absolute()
            .left(px(cm.x.clamp(8.0, max_x)))
            .top(px(cm.y.clamp(8.0, max_y)))
            .w(px(172.0))
            .p(px(5.0))
            .rounded(px(9.0))
            .bg(t.panel_bg)
            .border_1()
            .border_color(t.border_strong)
            .shadow_lg()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(menu_item(
                "tab-ctx-close",
                "关闭标签页 (⌘W)",
                false,
                Box::new(cx.listener(move |this, _, window, cx| {
                    this.data.tab_ctx_menu.open = false;
                    this.close_tab(&tid1, window, cx);
                })),
            ))
            .child(menu_item(
                "tab-ctx-close-others",
                "关闭其他标签页",
                false,
                Box::new(cx.listener(move |this, _, window, cx| {
                    this.data.tab_ctx_menu.open = false;
                    this.close_other_tabs(&tid2, window, cx);
                })),
            ))
            .child(menu_item(
                "tab-ctx-close-right",
                "关闭右侧标签页",
                false,
                Box::new(cx.listener(move |this, _, window, cx| {
                    this.data.tab_ctx_menu.open = false;
                    this.close_tabs_to_right(&tid3, window, cx);
                })),
            ))
            .child(div().h(px(1.0)).bg(t.border).my(px(4.0)))
            .child(menu_item(
                "tab-ctx-close-all",
                "关闭全部标签页",
                true,
                Box::new(cx.listener(move |this, _, window, cx| {
                    this.data.tab_ctx_menu.open = false;
                    this.close_all_tabs(window, cx);
                })),
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_objects(names: &[&str]) -> Vec<SchemaObject> {
        names
            .iter()
            .map(|name| SchemaObject {
                name: name.to_string(),
                schema: "public".into(),
                object_type: SchemaObjectType::Table,
                meta: String::new(),
            })
            .collect()
    }

    #[test]
    fn tree_ranges_preserve_group_counts_and_original_object_indices() {
        let catalog = Arc::new(BTreeMap::from([
            ("A".into(), sample_objects(&["hidden", "Find_1", "Find_2"])),
            ("B".into(), sample_objects(&["search", "FIND_3"])),
            ("C".into(), sample_objects(&[])),
        ]));
        let empty_favs = HashSet::new();
        // When searching, collapsed groups auto-expand so matches are visible.
        let tree = SidebarTree::new(
            Arc::clone(&catalog),
            " find ",
            &HashSet::from(["A".into()]),
            &empty_favs,
            false,
        );
        assert_eq!(tree.rows, 5);
        assert_eq!(tree.matched_objects, 3);
        assert_eq!(tree.total_objects, 5);
        assert!(matches!(tree.row(0), Some(TreeRow::Header(g)) if g.name == "A" && g.count == 2));
        assert!(matches!(tree.row(1), Some(TreeRow::Object(_, 1, obj)) if obj.name == "Find_1"));
        assert!(matches!(tree.row(2), Some(TreeRow::Object(_, 2, obj)) if obj.name == "Find_2"));
        assert!(matches!(tree.row(3), Some(TreeRow::Header(g)) if g.name == "B" && g.count == 1));
        assert!(matches!(tree.row(4), Some(TreeRow::Object(_, 1, obj)) if obj.name == "FIND_3"));
        assert!(tree.row(5).is_none());

        // When not searching, collapsed groups stay collapsed.
        let collapsed_tree = SidebarTree::new(
            Arc::clone(&catalog),
            "",
            &HashSet::from(["A".into()]),
            &empty_favs,
            false,
        );
        assert_eq!(collapsed_tree.rows, 4);
        assert!(matches!(collapsed_tree.row(0), Some(TreeRow::Header(g)) if g.name == "A" && g.count == 3));
        assert!(matches!(collapsed_tree.row(1), Some(TreeRow::Header(g)) if g.name == "B" && g.count == 2));

        let all = SidebarTree::new(Arc::clone(&catalog), "", &HashSet::new(), &empty_favs, false);
        assert_eq!(all.rows, 7);
        assert!(matches!(all.row(6), Some(TreeRow::Object(_, 1, obj)) if obj.name == "FIND_3"));
        assert!(SidebarTree::new(catalog, "absent", &HashSet::new(), &empty_favs, false)
            .row(0)
            .is_none());
    }

    #[test]
    fn tree_pins_favorites_and_supports_favorites_only_and_fuzzy_search() {
        let catalog = Arc::new(BTreeMap::from([(
            "数据表".into(),
            sample_objects(&["users", "user_orders", "audit_logs"]),
        )]));
        let favs = HashSet::from([("public".to_string(), "user_orders".to_string())]);

        let tree = SidebarTree::new(Arc::clone(&catalog), "", &HashSet::new(), &favs, false);
        // Header(收藏) + user_orders + Header(数据表) + 3 tables = 6 rows
        assert_eq!(tree.rows, 6);
        assert!(matches!(tree.row(0), Some(TreeRow::Header(g)) if g.is_favorites && g.count == 1));
        assert!(matches!(tree.row(1), Some(TreeRow::Object(_, _, obj)) if obj.name == "user_orders"));
        assert_eq!(
            tree.first_openable(),
            Some(("public".to_string(), "user_orders".to_string()))
        );

        let fav_only = SidebarTree::new(Arc::clone(&catalog), "", &HashSet::new(), &favs, true);
        assert_eq!(fav_only.rows, 2);
        assert_eq!(fav_only.matched_objects, 1);

        // Multi-token + fuzzy subsequence search
        assert!(matches_search_query("public", "user_orders", "usr ord"));
        assert!(matches_search_query("public", "user_orders", "pub.user"));
        assert!(matches_search_query("public", "user_orders", "uo"));
        assert!(!matches_search_query("public", "user_orders", "xyz"));
    }
}
