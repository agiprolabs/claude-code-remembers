use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{error, info};

use crate::daemon::DaemonState;
use crate::ipc::protocol::*;

// --- JSON-RPC types ---

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i64, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: None,
            }),
        }
    }
}

// --- MCP Tool/Resource definitions ---

fn tool_definitions() -> Value {
    json!({
        "tools": [
            {
                "name": "memory_remember",
                "description": "Store a memory/observation about the project. The daemon will classify, deduplicate, and index it. Use this to save architectural decisions, patterns, gotchas, preferences, or progress notes.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "The memory or observation to store"
                        }
                    },
                    "required": ["content"]
                }
            },
            {
                "name": "memory_recall",
                "description": "Search project memories by keyword or topic. Returns relevant memories ranked by importance and relevance.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query — keywords or topic to search for"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max results to return (default: 10)",
                            "default": 10
                        }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "memory_context",
                "description": "Get the full compressed project memory context. Returns a structured summary of all important memories organized by type (architecture, decisions, patterns, gotchas, preferences, progress).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "max_tokens": {
                            "type": "integer",
                            "description": "Maximum tokens for context (default: 1500)",
                            "default": 1500
                        }
                    }
                }
            },
            {
                "name": "memory_status",
                "description": "Get memory system status — total memories, counts by type, last consolidation time.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            }
        ]
    })
}

fn resource_definitions() -> Value {
    json!({
        "resources": [
            {
                "uri": "memory://context",
                "name": "Project Memory Context",
                "description": "Compressed, structured project memory context. Auto-loaded at session start.",
                "mimeType": "text/markdown"
            }
        ]
    })
}

// --- MCP Server ---

pub async fn serve_stdio(state: Arc<DaemonState>) {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut lines = BufReader::new(stdin).lines();

    info!("MCP server started on stdio");

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                let resp = JsonRpcResponse::error(
                    Value::Null,
                    -32700,
                    format!("Parse error: {e}"),
                );
                let _ = write_response(&mut stdout, &resp).await;
                continue;
            }
        };

        let id = request.id.clone().unwrap_or(Value::Null);
        let response = handle_mcp_request(&request, &state).await;

        // Notifications (no id) don't get responses
        if request.id.is_some() {
            let resp = match response {
                Ok(result) => JsonRpcResponse::success(id, result),
                Err((code, msg)) => JsonRpcResponse::error(id, code, msg),
            };
            if let Err(e) = write_response(&mut stdout, &resp).await {
                error!("Failed to write response: {e}");
                break;
            }
        }
    }

    info!("MCP server stdin closed, shutting down");
}

async fn write_response(
    stdout: &mut tokio::io::Stdout,
    resp: &JsonRpcResponse,
) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(resp).unwrap();
    bytes.push(b'\n');
    stdout.write_all(&bytes).await?;
    stdout.flush().await?;
    Ok(())
}

async fn handle_mcp_request(
    req: &JsonRpcRequest,
    state: &DaemonState,
) -> Result<Value, (i64, String)> {
    match req.method.as_str() {
        "initialize" => {
            Ok(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {},
                    "resources": {}
                },
                "serverInfo": {
                    "name": "claude-remember",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }))
        }

        "notifications/initialized" => {
            // Client acknowledges initialization — no response needed for notifications
            Ok(Value::Null)
        }

        "tools/list" => Ok(tool_definitions()),

        "tools/call" => {
            let params = req.params.as_ref().ok_or((
                -32602,
                "Missing params".to_string(),
            ))?;

            let tool_name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "Missing tool name".to_string()))?;

            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or(json!({}));

            handle_tool_call(tool_name, arguments, state).await
        }

        "resources/list" => Ok(resource_definitions()),

        "resources/read" => {
            let params = req.params.as_ref().ok_or((
                -32602,
                "Missing params".to_string(),
            ))?;

            let uri = params
                .get("uri")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "Missing uri".to_string()))?;

            handle_resource_read(uri, state).await
        }

        "ping" => Ok(json!({})),

        _ => Err((-32601, format!("Method not found: {}", req.method))),
    }
}

async fn handle_tool_call(
    name: &str,
    args: Value,
    state: &DaemonState,
) -> Result<Value, (i64, String)> {
    match name {
        "memory_remember" => {
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "Missing 'content' argument".to_string()))?;

            let resp = state
                .handle_ingest(IngestParams {
                    content: content.to_string(),
                    session_id: None,
                })
                .await;

            match resp {
                Response::Ok { data } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Memory stored (id: {}, deduplicated: {})",
                            data.get("memory_id").unwrap_or(&Value::Null),
                            data.get("deduplicated").unwrap_or(&Value::Bool(false))
                        )
                    }]
                })),
                Response::Error { message } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Error storing memory: {message}")
                    }],
                    "isError": true
                })),
            }
        }

        "memory_recall" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "Missing 'query' argument".to_string()))?;

            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            let resp = state.handle_search(SearchParams {
                query: query.to_string(),
                limit,
            });

            match resp {
                Response::Ok { data } => {
                    let ids: Vec<i64> = serde_json::from_value(data).unwrap_or_default();

                    if ids.is_empty() {
                        return Ok(json!({
                            "content": [{
                                "type": "text",
                                "text": "No matching memories found."
                            }]
                        }));
                    }

                    // Fetch full memories for the matched IDs
                    let conn = state.db.lock().unwrap();
                    let mut results = Vec::new();
                    for id in &ids {
                        if let Ok(mut stmt) = conn.prepare(
                            "SELECT id, content, summary, memory_type, importance FROM memories WHERE id = ?1"
                        ) {
                            if let Ok(mut rows) = stmt.query(rusqlite::params![id]) {
                                if let Ok(Some(row)) = rows.next() {
                                    let mid: i64 = row.get(0).unwrap_or(0);
                                    let content: String = row.get(1).unwrap_or_default();
                                    let summary: Option<String> = row.get(2).ok();
                                    let mtype: String = row.get(3).unwrap_or_default();
                                    let importance: f64 = row.get(4).unwrap_or(0.0);
                                    results.push(format!(
                                        "[#{mid}] [{mtype}] (importance: {importance:.1}) {}",
                                        summary.as_deref().unwrap_or(&content)
                                    ));
                                }
                            }
                        }
                    }
                    drop(conn);

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": if results.is_empty() {
                                "No matching memories found.".to_string()
                            } else {
                                results.join("\n")
                            }
                        }]
                    }))
                }
                Response::Error { message } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Search error: {message}")
                    }],
                    "isError": true
                })),
            }
        }

        "memory_context" => {
            let max_tokens = args
                .get("max_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(1500) as usize;

            let resp = state
                .handle_get_context(GetContextParams {
                    max_tokens,
                    session_id: None,
                })
                .await;

            match resp {
                Response::Ok { data } => {
                    let context = data
                        .get("context")
                        .and_then(|v| v.as_str())
                        .unwrap_or("No memories recorded yet.");

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": context
                        }]
                    }))
                }
                Response::Error { message } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Error: {message}")
                    }],
                    "isError": true
                })),
            }
        }

        "memory_status" => {
            let resp = state.handle_get_status();

            match resp {
                Response::Ok { data } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&data).unwrap_or_default()
                    }]
                })),
                Response::Error { message } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Error: {message}")
                    }],
                    "isError": true
                })),
            }
        }

        _ => Err((-32602, format!("Unknown tool: {name}"))),
    }
}

async fn handle_resource_read(
    uri: &str,
    state: &DaemonState,
) -> Result<Value, (i64, String)> {
    match uri {
        "memory://context" => {
            let resp = state
                .handle_get_context(GetContextParams {
                    max_tokens: 1500,
                    session_id: None,
                })
                .await;

            match resp {
                Response::Ok { data } => {
                    let context = data
                        .get("context")
                        .and_then(|v| v.as_str())
                        .unwrap_or("No memories recorded yet.")
                        .to_string();

                    Ok(json!({
                        "contents": [{
                            "uri": "memory://context",
                            "mimeType": "text/markdown",
                            "text": context
                        }]
                    }))
                }
                Response::Error { message } => Err((-32000, message)),
            }
        }

        _ => Err((-32602, format!("Unknown resource: {uri}"))),
    }
}
