use crate::adapter::DatabaseAdapter;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

pub async fn run_stdio_mcp(adapter: Arc<dyn DatabaseAdapter>, read_only: bool) -> std::io::Result<()> {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut lines = BufReader::new(stdin).lines();

    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: Value::Null,
                    result: None,
                    error: Some(json!({ "code": -32700, "message": format!("Parse error: {e}") })),
                };
                let out = serde_json::to_string(&resp).unwrap_or_default() + "\n";
                stdout.write_all(out.as_bytes()).await?;
                stdout.flush().await?;
                continue;
            }
        };

        let Some(id) = req.id.clone() else {
            // Notification (e.g. notifications/initialized), no response required
            continue;
        };

        let resp = handle_request(&adapter, read_only, id, &req.method, req.params).await;
        let out = serde_json::to_string(&resp).unwrap_or_default() + "\n";
        stdout.write_all(out.as_bytes()).await?;
        stdout.flush().await?;
    }

    Ok(())
}

async fn handle_request(
    adapter: &Arc<dyn DatabaseAdapter>,
    read_only: bool,
    id: Value,
    method: &str,
    params: Option<Value>,
) -> JsonRpcResponse {
    match method {
        "initialize" => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "velodb-mcp",
                    "version": "0.2.0"
                }
            })),
            error: None,
        },
        "tools/list" => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(json!({
                "tools": [
                    {
                        "name": "list_schemas",
                        "description": "List all user schemas and database objects in the connected database",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "schema": { "type": "string", "description": "Optional schema name to list objects for" }
                            }
                        }
                    },
                    {
                        "name": "describe_table",
                        "description": "Get columns, indexes, constraints, and DDL for a table",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "schema": { "type": "string", "default": "public" },
                                "table": { "type": "string" }
                            },
                            "required": ["table"]
                        }
                    },
                    {
                        "name": "run_read_query",
                        "description": "Execute a SQL query (enforces read-only lock when enabled)",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "sql": { "type": "string" }
                            },
                            "required": ["sql"]
                        }
                    },
                    {
                        "name": "explain_query",
                        "description": "Run EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) on a SQL query",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "sql": { "type": "string" }
                            },
                            "required": ["sql"]
                        }
                    }
                ]
            })),
            error: None,
        },
        "tools/call" => {
            let params = params.unwrap_or(Value::Null);
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));

            let call_result = match name {
                "list_schemas" => {
                    if let Some(schema) = args.get("schema").and_then(|v| v.as_str()) {
                        adapter
                            .list_schema_objects(schema)
                            .await
                            .map(|o| serde_json::to_string_pretty(&o).unwrap_or_default())
                    } else {
                        adapter
                            .list_schemas()
                            .await
                            .map(|s| serde_json::to_string_pretty(&s).unwrap_or_default())
                    }
                }
                "describe_table" => {
                    let schema = args.get("schema").and_then(|v| v.as_str()).unwrap_or("public");
                    let table = args.get("table").and_then(|v| v.as_str()).unwrap_or("");
                    adapter
                        .get_table_structure(schema, table)
                        .await
                        .map(|s| serde_json::to_string_pretty(&s).unwrap_or_default())
                }
                "run_read_query" => {
                    let sql = args.get("sql").and_then(|v| v.as_str()).unwrap_or("");
                    adapter
                        .execute_sql(sql, read_only)
                        .await
                        .map(|r| serde_json::to_string_pretty(&r).unwrap_or_default())
                }
                "explain_query" => {
                    let sql = args.get("sql").and_then(|v| v.as_str()).unwrap_or("");
                    adapter
                        .explain_sql(sql)
                        .await
                        .map(|p| serde_json::to_string_pretty(&p).unwrap_or_default())
                }
                _ => {
                    return JsonRpcResponse {
                        jsonrpc: "2.0",
                        id,
                        result: None,
                        error: Some(json!({ "code": -32601, "message": format!("Unknown tool: {name}") })),
                    };
                }
            };

            match call_result {
                Ok(text) => JsonRpcResponse {
                    jsonrpc: "2.0",
                    id,
                    result: Some(json!({
                        "content": [{ "type": "text", "text": text }]
                    })),
                    error: None,
                },
                Err(e) => JsonRpcResponse {
                    jsonrpc: "2.0",
                    id,
                    result: Some(json!({
                        "isError": true,
                        "content": [{ "type": "text", "text": e.to_string() }]
                    })),
                    error: None,
                },
            }
        }
        _ => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(json!({ "code": -32601, "message": format!("Method not found: {method}") })),
        },
    }
}
