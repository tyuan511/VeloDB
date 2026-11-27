use crate::adapter::{ColumnDef, DbDriver, SchemaObject};
use anyhow::Result;
use gpui::{App, Task, Window};
use gpui_component::input::{CompletionProvider, HoverProvider, Rope};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, Documentation,
    Hover, HoverContents, MarkupContent, MarkupKind,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock, RwLock};

// Database identifiers cannot contain NUL. Separators allow a single substring
// search across a group while preventing matches from crossing object boundaries.
struct ObjectNameIndex {
    text: String,
    ends: Vec<usize>,
}

impl ObjectNameIndex {
    fn new(objects: &[SchemaObject]) -> Self {
        let capacity = 1 + objects.iter().map(|obj| obj.name.len() + 1).sum::<usize>();
        let mut text = String::with_capacity(capacity);
        let mut ends = Vec::with_capacity(objects.len());
        text.push('\0');
        for obj in objects {
            if obj.name.is_ascii() {
                let start = text.len();
                text.push_str(&obj.name);
                text[start..].make_ascii_lowercase();
            } else {
                text.push_str(&obj.name.to_lowercase());
            }
            text.push('\0');
            ends.push(text.len());
        }
        Self { text, ends }
    }

    fn matching<'a>(&'a self, needle: &'a str) -> impl Iterator<Item = usize> + 'a {
        let mut start = 1;
        std::iter::from_fn(move || {
            let pos = start + self.text[start..].find(needle)?;
            let row = self.ends.partition_point(|&end| end <= pos);
            start = self.ends[row];
            Some(row)
        })
    }

    fn exact(&self, delimited: &str) -> Option<usize> {
        let start = self.text.find(delimited)? + 1;
        Some(self.ends.partition_point(|&end| end <= start))
    }
}

#[derive(Default)]
pub struct SqlLspCatalog {
    pub driver: Option<DbDriver>,
    pub current_db: String,
    pub current_schema: String,
    pub databases: Vec<String>,
    pub schemas: Vec<String>,
    objects: Arc<BTreeMap<String, Vec<SchemaObject>>>,
    object_names: Arc<OnceLock<Vec<ObjectNameIndex>>>,
    pub columns_by_table: HashMap<String, Arc<[ColumnDef]>>,
}

impl SqlLspCatalog {
    pub fn update_schema_objects(
        &mut self,
        driver: DbDriver,
        current_db: String,
        current_schema: String,
        databases: Vec<String>,
        schemas: Vec<String>,
        groups: Arc<BTreeMap<String, Vec<SchemaObject>>>,
    ) {
        self.driver = Some(driver);
        self.current_db = current_db;
        self.current_schema = current_schema;
        self.databases = databases;
        self.schemas = schemas;
        if !Arc::ptr_eq(&self.objects, &groups) {
            self.objects = groups;
            self.object_names = Arc::default();
        }
    }

    fn build_object_index(groups: &BTreeMap<String, Vec<SchemaObject>>) -> Vec<ObjectNameIndex> {
        groups
            .values()
            .map(|group| ObjectNameIndex::new(group))
            .collect()
    }

    fn indexed_groups(&self) -> impl Iterator<Item = (&[SchemaObject], &ObjectNameIndex)> {
        let names = self
            .object_names
            .get_or_init(|| Self::build_object_index(&self.objects));
        self.objects.values().map(Vec::as_slice).zip(names)
    }

    fn matching_objects<'a>(
        &'a self,
        prefix: &'a str,
    ) -> Box<dyn Iterator<Item = &'a SchemaObject> + 'a> {
        if prefix.is_empty() {
            return Box::new(self.objects.values().flatten());
        }
        Box::new(
            self.indexed_groups()
                .flat_map(move |(objects, names)| names.matching(prefix).map(|row| &objects[row])),
        )
    }

    fn object_named(&self, lower: &str) -> Option<&SchemaObject> {
        let delimited = format!("\0{lower}\0");
        self.indexed_groups()
            .find_map(|(objects, names)| names.exact(&delimited).map(|row| &objects[row]))
    }

    pub fn update_table_columns(&mut self, table: &str, columns: Arc<[ColumnDef]>) {
        self.columns_by_table.insert(table.to_lowercase(), columns);
    }
}

pub struct SqlLspProvider {
    pub catalog: Arc<RwLock<SqlLspCatalog>>,
}

impl SqlLspProvider {
    pub fn new(catalog: Arc<RwLock<SqlLspCatalog>>) -> Self {
        Self { catalog }
    }

    fn lookup<T: Send + 'static>(
        &self,
        needs_index: bool,
        stale: T,
        cx: &App,
        query: impl FnOnce(&SqlLspCatalog) -> T + Send + 'static,
    ) -> Task<Result<T>> {
        let cat = self.catalog.read().unwrap_or_else(|e| e.into_inner());
        if !needs_index || cat.objects.is_empty() || cat.object_names.get().is_some() {
            return Task::ready(Ok(query(&cat)));
        }

        let groups = Arc::clone(&cat.objects);
        let index = Arc::clone(&cat.object_names);
        let catalog = Arc::clone(&self.catalog);
        // Build outside the catalog lock so schema changes can proceed during a cold lookup.
        cx.background_executor().spawn(async move {
            index.get_or_init(|| SqlLspCatalog::build_object_index(&groups));
            let cat = catalog.read().unwrap_or_else(|e| e.into_inner());
            if Arc::ptr_eq(&cat.object_names, &index) {
                Ok(query(&cat))
            } else {
                Ok(stale)
            }
        })
    }
}

const SQL_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "AND",
    "OR",
    "NOT",
    "IN",
    "IS NULL",
    "IS NOT NULL",
    "LIKE",
    "BETWEEN",
    "JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "INNER JOIN",
    "OUTER JOIN",
    "ON",
    "AS",
    "DISTINCT",
    "GROUP BY",
    "HAVING",
    "ORDER BY",
    "ASC",
    "DESC",
    "LIMIT",
    "OFFSET",
    "UNION",
    "UNION ALL",
    "INSERT INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE FROM",
    "CREATE TABLE",
    "ALTER TABLE",
    "DROP TABLE",
    "EXPLAIN",
    "SHOW TABLES",
    "SHOW DATABASES",
    "DESCRIBE",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "WITH",
    "EXISTS",
];

const SQL_FUNCTIONS: &[(&str, &str)] = &[
    ("COUNT(*)", "Aggregate row count"),
    ("SUM()", "Aggregate sum"),
    ("AVG()", "Aggregate average"),
    ("MIN()", "Aggregate minimum"),
    ("MAX()", "Aggregate maximum"),
    ("COALESCE()", "Return first non-NULL expression"),
    ("IFNULL()", "MySQL: return fallback when NULL"),
    ("CONCAT()", "Concatenate strings"),
    ("SUBSTRING()", "Extract substring"),
    ("LOWER()", "Convert to lowercase"),
    ("UPPER()", "Convert to uppercase"),
    ("TRIM()", "Strip leading/trailing whitespace"),
    ("NOW()", "Current timestamp"),
    ("DATABASE()", "Current MySQL database name"),
    ("CURRENT_SCHEMA()", "Current PostgreSQL schema name"),
    ("JSON_EXTRACT()", "Extract value from JSON"),
];

fn extract_prefix_and_qualifier(text: &Rope, offset: usize) -> (Option<String>, String) {
    let end = text.floor_char_boundary(offset.min(text.len()));
    let len = text
        .bytes_at(end)
        .reversed()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'.' | b'`' | b'"'))
        .count();
    let token = text.slice(end - len..end).to_string();
    if let Some(dot_pos) = token.rfind('.') {
        let qualifier = token[..dot_pos]
            .trim_matches(|c| c == '`' || c == '"')
            .to_lowercase();
        let prefix = token[dot_pos + 1..]
            .trim_matches(|c| c == '`' || c == '"')
            .to_lowercase();
        (Some(qualifier), prefix)
    } else {
        let prefix = token.trim_matches(|c| c == '`' || c == '"').to_lowercase();
        (None, prefix)
    }
}

fn word_at_offset(text: &Rope, offset: usize) -> Option<String> {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    let is_ident = |b: &u8| b.is_ascii_alphanumeric() || *b == b'_';
    let before = text
        .bytes_at(offset)
        .reversed()
        .take_while(is_ident)
        .count();
    let after = text.bytes_at(offset).take_while(is_ident).count();
    (before + after > 0).then(|| text.slice(offset - before..offset + after).to_string())
}

impl SqlLspCatalog {
    fn complete(&self, qualifier: Option<String>, prefix: &str) -> CompletionResponse {
        let mut items: Vec<CompletionItem> = Vec::new();

        if let Some(qual) = qualifier {
            // 1. Table-qualified column completion: `table_name.`
            if let Some(cols) = self.columns_by_table.get(&qual) {
                for col in cols.iter() {
                    if items.len() == 40 {
                        break;
                    }
                    if prefix.is_empty() || col.name.to_lowercase().contains(prefix) {
                        let pk_mark = if col.is_pk { " · PK" } else { "" };
                        let fk_mark = if let Some(ref ft) = col.fk_table {
                            format!(" · FK → {ft}")
                        } else {
                            String::new()
                        };
                        items.push(CompletionItem {
                            label: col.name.clone(),
                            kind: Some(CompletionItemKind::FIELD),
                            detail: Some(format!("{}{pk_mark}{fk_mark}", col.data_type)),
                            documentation: col
                                .comment
                                .as_ref()
                                .map(|c| Documentation::String(c.clone())),
                            ..Default::default()
                        });
                    }
                }
            }
            // 2. Database/Schema-qualified table completion: `db_name.`
            if qual == self.current_db.to_lowercase()
                || qual == self.current_schema.to_lowercase()
                || self.databases.iter().any(|d| d.to_lowercase() == qual)
                || self.schemas.iter().any(|s| s.to_lowercase() == qual)
            {
                for obj in self.matching_objects(prefix).take(40 - items.len()) {
                    items.push(CompletionItem {
                        label: obj.name.clone(),
                        kind: Some(CompletionItemKind::STRUCT),
                        detail: Some(format!("{} · {}", obj.schema, obj.meta)),
                        ..Default::default()
                    });
                }
            }
        } else {
            // 1. Tables / Views / Functions in current DB/Schema
            for obj in self.matching_objects(prefix).take(40 - items.len()) {
                let kind = match obj.object_type {
                    crate::adapter::SchemaObjectType::Function => CompletionItemKind::FUNCTION,
                    crate::adapter::SchemaObjectType::View
                    | crate::adapter::SchemaObjectType::MaterializedView => {
                        CompletionItemKind::INTERFACE
                    }
                    _ => CompletionItemKind::STRUCT,
                };
                items.push(CompletionItem {
                    label: obj.name.clone(),
                    kind: Some(kind),
                    detail: Some(format!("{} · {}", obj.schema, obj.meta)),
                    ..Default::default()
                });
            }

            // 2. Cached columns from opened tables
            let mut seen_cols = std::collections::HashSet::new();
            for (tbl, cols) in &self.columns_by_table {
                if items.len() == 40 {
                    break;
                }
                for col in cols.iter() {
                    if items.len() == 40 {
                        break;
                    }
                    if (prefix.is_empty() || col.name.to_lowercase().contains(prefix))
                        && seen_cols.insert((tbl.as_str(), col.name.as_str()))
                    {
                        items.push(CompletionItem {
                            label: col.name.clone(),
                            kind: Some(CompletionItemKind::FIELD),
                            detail: Some(format!("{tbl} ({})", col.data_type)),
                            ..Default::default()
                        });
                    }
                }
            }

            // 3. Databases / Schemas
            for db in &self.databases {
                if items.len() == 40 {
                    break;
                }
                if !prefix.is_empty() && db.to_lowercase().contains(prefix) {
                    items.push(CompletionItem {
                        label: db.clone(),
                        kind: Some(CompletionItemKind::MODULE),
                        detail: Some("Database".to_string()),
                        ..Default::default()
                    });
                }
            }

            // 4. SQL Keywords
            for &kw in SQL_KEYWORDS {
                if items.len() == 40 {
                    break;
                }
                if prefix.is_empty() || kw.to_lowercase().starts_with(prefix) {
                    items.push(CompletionItem {
                        label: kw.to_string(),
                        kind: Some(CompletionItemKind::KEYWORD),
                        detail: Some("SQL Keyword".to_string()),
                        ..Default::default()
                    });
                }
            }

            // 5. SQL Functions
            for &(func, doc) in SQL_FUNCTIONS {
                if items.len() == 40 {
                    break;
                }
                if prefix.is_empty() || func.to_lowercase().starts_with(prefix) {
                    items.push(CompletionItem {
                        label: func.to_string(),
                        kind: Some(CompletionItemKind::FUNCTION),
                        detail: Some(doc.to_string()),
                        ..Default::default()
                    });
                }
            }
        }

        CompletionResponse::Array(items)
    }
}

impl CompletionProvider for SqlLspProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let (qualifier, prefix) = extract_prefix_and_qualifier(text, offset);
        self.lookup(
            !prefix.is_empty(),
            CompletionResponse::Array(Vec::new()),
            cx,
            move |cat| cat.complete(qualifier, &prefix),
        )
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
        new_text == "."
            || new_text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !new_text.is_empty()
    }
}

impl SqlLspCatalog {
    fn hover_word(&self, lower: &str) -> Option<Hover> {
        // Check if hovering over a table/view/function
        if let Some(obj) = self.object_named(lower) {
            let mut md = format!(
                "### `{}.{}`\n\n- **信息**: {}",
                obj.schema, obj.name, obj.meta
            );
            if let Some(cols) = self.columns_by_table.get(lower) {
                md.push_str("\n\n**字段:**\n");
                for c in cols.iter().take(12) {
                    let pk = if c.is_pk { " *(PK)*" } else { "" };
                    md.push_str(&format!("- `{}`: `{}`{pk}\n", c.name, c.data_type));
                }
                if cols.len() > 12 {
                    md.push_str(&format!("- *...共 {} 个字段*", cols.len()));
                }
            }
            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: md,
                }),
                range: None,
            });
        }

        // Check if hovering over a cached column
        for (tbl, cols) in &self.columns_by_table {
            if let Some(col) = cols.iter().find(|c| c.name.to_lowercase() == lower) {
                let mut md = format!(
                    "### `{tbl}.{}`\n\n- **类型**: `{}`\n- **可空**: {}",
                    col.name,
                    col.data_type,
                    if col.nullable { "YES" } else { "NO" }
                );
                if col.is_pk {
                    md.push_str("\n- **主键**: PRIMARY KEY");
                }
                if let Some(ref ft) = col.fk_table {
                    md.push_str(&format!(
                        "\n- **外键**: → `{ft}.{}`",
                        col.fk_column.as_deref().unwrap_or("")
                    ));
                }
                if let Some(ref cmt) = col.comment {
                    md.push_str(&format!("\n- **注释**: {cmt}"));
                }
                return Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: md,
                    }),
                    range: None,
                });
            }
        }

        None
    }
}

impl HoverProvider for SqlLspProvider {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<Hover>>> {
        let Some(word) = word_at_offset(text, offset) else {
            return Task::ready(Ok(None));
        };
        self.lookup(true, None, cx, move |cat| {
            cat.hover_word(&word.to_lowercase())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_search_preserves_catalog_order_and_refreshes_with_the_schema() {
        let group = |names: &[&str]| {
            Arc::new(BTreeMap::from([(
                "tables".into(),
                names
                    .iter()
                    .map(|&name| SchemaObject {
                        name: name.into(),
                        schema: "public".into(),
                        object_type: crate::adapter::SchemaObjectType::Table,
                        meta: String::new(),
                    })
                    .collect(),
            )]))
        };
        let mut cat = SqlLspCatalog::default();
        let update = |cat: &mut SqlLspCatalog, names: &[&str]| {
            cat.update_schema_objects(
                DbDriver::Postgres,
                "db".into(),
                "public".into(),
                vec![],
                vec![],
                group(names),
            );
        };
        update(&mut cat, &["FooFoo", "Bar", "Key", "FOO", "İx"]);
        assert_eq!(cat.matching_objects("").count(), 5);
        assert!(cat.object_names.get().is_none());
        let names = cat
            .matching_objects("oo")
            .map(|o| o.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["FooFoo", "FOO"]);
        assert!(cat.matching_objects("ob").next().is_none());
        assert_eq!(cat.object_named("foo").unwrap().name, "FOO");
        assert_eq!(cat.object_named("key").unwrap().name, "Key");
        assert_eq!(cat.object_named("i\u{307}x").unwrap().name, "İx");
        update(&mut cat, &["Replacement"]);
        assert!(cat.object_named("foo").is_none());
        assert_eq!(
            cat.matching_objects("place").next().unwrap().name,
            "Replacement"
        );
    }

    #[test]
    fn completion_and_hover_read_the_identifier_at_a_byte_offset() {
        let text = Rope::from("查询：`Orders`.ID 后缀");
        let end = "查询：`Orders`.ID".len();
        assert_eq!(
            extract_prefix_and_qualifier(&text, end),
            (Some("orders".into()), "id".into())
        );
        assert_eq!(word_at_offset(&text, end - 1).as_deref(), Some("ID"));
        assert_eq!(word_at_offset(&text, 1), None);
    }
}
