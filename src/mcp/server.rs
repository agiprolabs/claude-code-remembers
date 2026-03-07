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
            },
            {
                "name": "memory_session_end",
                "description": "Call at the end of a session with a summary of what happened. The daemon extracts multiple memories from the summary, capturing things that weren't explicitly remembered during the session.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "summary": {
                            "type": "string",
                            "description": "A summary of what happened in this session — what was built, decided, learned, or changed"
                        },
                        "session_id": {
                            "type": "string",
                            "description": "Optional session identifier"
                        }
                    },
                    "required": ["summary"]
                }
            },
            {
                "name": "memory_consolidate",
                "description": "Manually trigger a memory consolidation pass. Haiku analyzes unconsolidated memories, finds connections, generates insights, merges duplicates, and removes obsolete entries.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "memory_configure",
                "description": "Update daemon runtime settings. Currently supports consolidation_interval_secs (how often memories are consolidated, in seconds). Example: 300 = 5 min, 1800 = 30 min, 3600 = 1 hour.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "consolidation_interval_secs": {
                            "type": "integer",
                            "description": "Consolidation loop interval in seconds (e.g. 300 = 5 min, 1800 = 30 min)"
                        }
                    }
                }
            },
            {
                "name": "memory_feedback",
                "description": "Provide feedback on a recalled memory to adjust its importance. Use after memory_recall when results are helpful (+1) or not useful (-1).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "memory_id": {
                            "type": "integer",
                            "description": "The memory ID (from recall results, e.g. #42)"
                        },
                        "helpful": {
                            "type": "boolean",
                            "description": "true = boost importance, false = reduce importance"
                        }
                    },
                    "required": ["memory_id", "helpful"]
                }
            },
            {
                "name": "memory_delete",
                "description": "Delete a memory by ID. Use when a memory is wrong, outdated, or no longer relevant.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "memory_id": {
                            "type": "integer",
                            "description": "The memory ID to delete"
                        }
                    },
                    "required": ["memory_id"]
                }
            },
            {
                "name": "memory_list",
                "description": "List all memories without requiring a search query. Optionally filter by type.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": {
                            "type": "integer",
                            "description": "Max memories to return (default: 50)",
                            "default": 50
                        },
                        "memory_type": {
                            "type": "string",
                            "description": "Filter by type: architecture, decision, pattern, gotcha, preference, progress"
                        }
                    }
                }
            },
            {
                "name": "memory_update",
                "description": "Update the content of an existing memory. Re-generates the summary via Haiku.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "memory_id": {
                            "type": "integer",
                            "description": "The memory ID to update"
                        },
                        "content": {
                            "type": "string",
                            "description": "The new content for this memory"
                        }
                    },
                    "required": ["memory_id", "content"]
                }
            },
            {
                "name": "memory_export",
                "description": "Export all memories as JSON for backup or portability.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "memory_import",
                "description": "Import memories from a JSON array. Each item should have a 'content' field. Memories are processed through the full ingest pipeline (Haiku classification, dedup, semantic tags).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "memories": {
                            "type": "array",
                            "description": "Array of memory objects, each with at least a 'content' field",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "content": {
                                        "type": "string"
                                    }
                                },
                                "required": ["content"]
                            }
                        }
                    },
                    "required": ["memories"]
                }
            },
            {
                "name": "memory_setup",
                "description": "Generate the CLAUDE.md snippet that enables automatic memory tool usage for any project. Returns the text to add to the top of a project's CLAUDE.md file.",
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
                    "resources": {},
                    "prompts": {}
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

        "prompts/list" => Ok(json!({
            "prompts": [{
                "name": "memory_init",
                "description": "Load project memory context at session start"
            }]
        })),

        "prompts/get" => {
            let params = req.params.as_ref().ok_or((
                -32602,
                "Missing params".to_string(),
            ))?;
            let prompt_name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "Missing prompt name".to_string()))?;

            match prompt_name {
                "memory_init" => {
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
                                .unwrap_or("No memories recorded yet.");

                            Ok(json!({
                                "description": "Project memory context",
                                "messages": [{
                                    "role": "user",
                                    "content": {
                                        "type": "text",
                                        "text": format!("Here is the project memory context from previous sessions. Use this to understand the project better:\n\n{context}")
                                    }
                                }]
                            }))
                        }
                        Response::Error { message } => Err((-32000, message)),
                    }
                }
                _ => Err((-32602, format!("Unknown prompt: {prompt_name}"))),
            }
        }

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

        "memory_session_end" => {
            let summary = args
                .get("summary")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "Missing 'summary' argument".to_string()))?;

            let session_id = args
                .get("session_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let resp = state
                .handle_session_end_summary(summary, session_id.as_deref())
                .await;

            match resp {
                Response::Ok { data } => {
                    let count = data
                        .get("memories_extracted")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": format!("Session ended. Extracted {count} memories from summary.")
                        }]
                    }))
                }
                Response::Error { message } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Error processing session end: {message}")
                    }],
                    "isError": true
                })),
            }
        }

        "memory_consolidate" => {
            let resp = state.handle_consolidate().await;

            match resp {
                Response::Ok { data } => {
                    let msg = data
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Consolidation complete");

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": msg
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

        "memory_configure" => {
            let interval = args
                .get("consolidation_interval_secs")
                .and_then(|v| v.as_u64());

            let resp = state.handle_configure(interval);

            match resp {
                Response::Ok { data } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Configuration updated: {}", serde_json::to_string_pretty(&data).unwrap_or_default())
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

        "memory_feedback" => {
            let memory_id = args
                .get("memory_id")
                .and_then(|v| v.as_i64())
                .ok_or((-32602, "Missing 'memory_id' argument".to_string()))?;

            let helpful = args
                .get("helpful")
                .and_then(|v| v.as_bool())
                .ok_or((-32602, "Missing 'helpful' argument".to_string()))?;

            let resp = state.handle_feedback(memory_id, helpful);

            match resp {
                Response::Ok { .. } => {
                    let direction = if helpful { "boosted" } else { "reduced" };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": format!("Memory #{memory_id} importance {direction}.")
                        }]
                    }))
                }
                Response::Error { message } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Error updating feedback: {message}")
                    }],
                    "isError": true
                })),
            }
        }

        "memory_delete" => {
            let memory_id = args
                .get("memory_id")
                .and_then(|v| v.as_i64())
                .ok_or((-32602, "Missing 'memory_id' argument".to_string()))?;

            let resp = state.handle_delete(memory_id);

            match resp {
                Response::Ok { .. } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Memory #{memory_id} deleted.")
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

        "memory_list" => {
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(50) as usize;

            let memory_type = args
                .get("memory_type")
                .and_then(|v| v.as_str());

            let resp = state.handle_list(limit, memory_type);

            match resp {
                Response::Ok { data } => {
                    let text = data
                        .get("memories")
                        .and_then(|v| v.as_str())
                        .unwrap_or("No memories found.");
                    let count = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": format!("{count} memories:\n{text}")
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

        "memory_update" => {
            let memory_id = args
                .get("memory_id")
                .and_then(|v| v.as_i64())
                .ok_or((-32602, "Missing 'memory_id' argument".to_string()))?;

            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "Missing 'content' argument".to_string()))?;

            let resp = state.handle_update(memory_id, content).await;

            match resp {
                Response::Ok { .. } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Memory #{memory_id} updated.")
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

        "memory_export" => {
            let resp = state.handle_export();

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

        "memory_import" => {
            let items = args
                .get("memories")
                .and_then(|v| v.as_array())
                .ok_or((-32602, "Missing 'memories' array argument".to_string()))?;

            let resp = state.handle_import(items).await;

            match resp {
                Response::Ok { data } => {
                    let count = data.get("imported").and_then(|v| v.as_u64()).unwrap_or(0);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": format!("Imported {count} memories.")
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

        "memory_setup" => {
            let resp = state.handle_setup();

            match resp {
                Response::Ok { data } => {
                    let snippet = data
                        .get("snippet")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let instructions = data
                        .get("instructions")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": format!("{instructions}\n\n```markdown\n{snippet}\n```")
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
