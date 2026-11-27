pub mod sync;

use crate::adapter::ConnectionConfig;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

pub const MIN_FONT_SIZE: u16 = 11;
pub const MAX_FONT_SIZE: u16 = 16;

fn default_font_size() -> u16 {
    13
}

fn default_sidebar_width() -> f32 {
    256.0
}

fn default_inspector_width() -> f32 {
    360.0
}

fn default_sql_editor_height() -> f32 {
    260.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub theme: String,        // "system" | "light" | "dark"
    pub font_family: String,
    #[serde(default = "default_font_size")]
    pub font_size: u16,
    #[serde(default = "default_font_size")]
    pub grid_font_size: u16,
    pub row_density: String,  // "compact" | "comfortable" | "spacious"
    pub vibrancy: bool,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    #[serde(default = "default_inspector_width")]
    pub inspector_width: f32,
    #[serde(default = "default_sql_editor_height")]
    pub sql_editor_height: f32,
    pub default_limit: usize,
    pub null_label: String,
    pub timezone: String,     // "UTC" | "Local"
    pub thousand_sep: bool,
    pub show_fk_jump: bool,
    #[serde(default = "default_font_size")]
    pub editor_font_size: u16,
    pub show_line_numbers: bool,
    pub tab_size: u8,
    pub auto_uppercase: bool,
    pub run_behavior: String, // "statement" | "all"
    pub safe_mode: bool,
    pub prod_confirm: bool,
    pub query_timeout_secs: u64,
    pub keep_alive: bool,
    pub vim_mode: bool,
    pub keymap_preset: String, // "macos" | "datagrip"
    pub mcp_enabled: bool,
    pub mcp_read_only: bool,
    pub mcp_transport: String, // "stdio" | "sse"
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "system".to_string(),
            font_family: "SF Mono".to_string(),
            font_size: default_font_size(),
            grid_font_size: default_font_size(),
            row_density: "comfortable".to_string(),
            vibrancy: true,
            sidebar_width: default_sidebar_width(),
            inspector_width: default_inspector_width(),
            sql_editor_height: default_sql_editor_height(),
            default_limit: 100,
            null_label: "NULL".to_string(),
            timezone: "UTC".to_string(),
            thousand_sep: false,
            show_fk_jump: true,
            editor_font_size: default_font_size(),
            show_line_numbers: true,
            tab_size: 2,
            auto_uppercase: true,
            run_behavior: "statement".to_string(),
            safe_mode: true,
            prod_confirm: true,
            query_timeout_secs: 30,
            keep_alive: true,
            vim_mode: false,
            keymap_preset: "macos".to_string(),
            mcp_enabled: true,
            mcp_read_only: true,
            mcp_transport: "stdio".to_string(),
        }
    }
}

impl AppSettings {
    pub fn effective_font_size(&self) -> f32 {
        self.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE) as f32
    }

    pub fn set_global_font_size(&mut self, size: u16) {
        let clamped = size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        self.font_size = clamped;
        self.grid_font_size = clamped;
        self.editor_font_size = clamped;
    }

    pub fn to_toml_string(&self) -> String {
        format!(
            "# ~/.config/velodb/settings.toml\n\
             [appearance]\n\
             theme = \"{}\"\n\
             font_family = \"{}\"\n\
             font_size = {}\n\
             row_density = \"{}\"\n\
             vibrancy = {}\n\
             grid_font_size = {}\n\n\
             [grid]\n\
             default_limit = {}\n\
             null_label = \"{}\"\n\
             timezone = \"{}\"\n\
             thousand_separator = {}\n\
             fk_jump_links = {}\n\n\
             [editor]\n\
             font_size = {}\n\
             line_numbers = {}\n\
             tab_size = {}\n\
             auto_uppercase_keywords = {}\n\
             run_behavior = \"{}\"\n\
             vim_mode = {}\n\n\
             [safety]\n\
             safe_mode = {}\n\
             prod_confirm = {}\n\
             query_timeout_secs = {}\n\
             keep_alive = {}\n\n\
             [keybindings]\n\
             preset = \"{}\"\n\n\
             [mcp]\n\
             enabled = {}\n\
             read_only = {}\n\
             transport = \"{}\"\n",
            self.theme,
            self.font_family,
            self.font_size,
            self.row_density,
            self.vibrancy,
            self.grid_font_size,
            self.default_limit,
            self.null_label,
            self.timezone,
            self.thousand_sep,
            self.show_fk_jump,
            self.editor_font_size,
            self.show_line_numbers,
            self.tab_size,
            self.auto_uppercase,
            self.run_behavior,
            self.vim_mode,
            self.safe_mode,
            self.prod_confirm,
            self.query_timeout_secs,
            self.keep_alive,
            self.keymap_preset,
            self.mcp_enabled,
            self.mcp_read_only,
            self.mcp_transport,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlSnippet {
    pub id: String,
    pub name: String,
    pub sql: Arc<str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryHistoryEntry {
    pub time: String,
    pub elapsed_ms: String,
    pub sql: Arc<str>,
}

pub fn config_dir() -> PathBuf {
    let base = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("velodb");
    let _ = fs::create_dir_all(&base);
    base
}

pub fn load_settings() -> AppSettings {
    let path = config_dir().join("settings.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(settings: &AppSettings) {
    let dir = config_dir();
    let _ = fs::write(
        dir.join("settings.json"),
        serde_json::to_string_pretty(settings).unwrap_or_default(),
    );
    let _ = fs::write(dir.join("settings.toml"), settings.to_toml_string());
}

pub fn load_connections() -> Vec<ConnectionConfig> {
    let path = config_dir().join("connections.json");
    if let Ok(s) = fs::read_to_string(&path) {
        if let Ok(list) = serde_json::from_str::<Vec<ConnectionConfig>>(&s) {
            return list;
        }
    }
    Vec::new()
}

pub fn save_connections(conns: &[ConnectionConfig]) {
    let path = config_dir().join("connections.json");
    let _ = fs::write(
        path,
        serde_json::to_string_pretty(conns).unwrap_or_default(),
    );
}

pub fn default_snippets() -> Vec<SqlSnippet> {
    vec![
        SqlSnippet {
            id: "ltv".into(),
            name: "Top LTV Users".into(),
            sql: "SELECT\n  u.id,\n  u.email,\n  COUNT(o.id) AS orders,\n  SUM(o.amount_cents) / 100.0 AS ltv_usd\nFROM public.users u\nLEFT JOIN public.orders o ON o.user_id = u.id\nGROUP BY 1, 2\nORDER BY ltv_usd DESC\nLIMIT 50;".into(),
        },
        SqlSnippet {
            id: "slow".into(),
            name: "Slow Join Explain".into(),
            sql: "EXPLAIN (ANALYZE, BUFFERS)\nSELECT *\nFROM public.orders o\nJOIN public.users u ON u.id = o.user_id\nWHERE o.amount_cents > 5000;".into(),
        },
        SqlSnippet {
            id: "locks".into(),
            name: "Active Sessions & Locks".into(),
            sql: "SELECT pid, usename, state, wait_event_type, LEFT(query, 80) AS query\nFROM pg_stat_activity\nWHERE state != 'idle'\nORDER BY query_start;".into(),
        },
        SqlSnippet {
            id: "sizes".into(),
            name: "Table Sizes (Data + Index)".into(),
            sql: "SELECT schemaname, relname, pg_size_pretty(pg_total_relation_size(relid)) AS total_size\nFROM pg_stat_user_tables\nORDER BY pg_total_relation_size(relid) DESC\nLIMIT 20;".into(),
        },
    ]
}

pub fn load_snippets() -> Vec<SqlSnippet> {
    let path = config_dir().join("snippets.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(default_snippets)
}

pub fn save_snippets(snips: &[SqlSnippet]) {
    let path = config_dir().join("snippets.json");
    let _ = fs::write(
        path,
        serde_json::to_string_pretty(snips).unwrap_or_default(),
    );
}

pub fn load_history() -> Vec<QueryHistoryEntry> {
    let path = config_dir().join("history.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_history(hist: &[QueryHistoryEntry]) {
    let path = config_dir().join("history.json");
    let _ = fs::write(
        path,
        serde_json::to_string_pretty(hist).unwrap_or_default(),
    );
}

pub fn load_favorites() -> std::collections::HashSet<String> {
    let path = config_dir().join("favorites.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .map(|list| list.into_iter().collect())
        .unwrap_or_default()
}

pub fn save_favorites(favorites: &std::collections::HashSet<String>) {
    let path = config_dir().join("favorites.json");
    let mut sorted: Vec<&String> = favorites.iter().collect();
    sorted.sort();
    let _ = fs::write(
        path,
        serde_json::to_string_pretty(&sorted).unwrap_or_default(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_queries_keep_the_existing_json_string_format() {
        let sql = "SELECT '雪';\nSELECT 2;";
        let history = serde_json::json!({"time": "09:00", "elapsed_ms": "1 ms", "sql": sql});
        let snippet = serde_json::json!({"id": "example", "name": "demo", "sql": sql});
        let entry: QueryHistoryEntry = serde_json::from_value(history.clone()).unwrap();
        let saved: SqlSnippet = serde_json::from_value(snippet.clone()).unwrap();
        assert_eq!(serde_json::to_value(entry).unwrap(), history);
        assert_eq!(serde_json::to_value(saved).unwrap(), snippet);
    }
}
