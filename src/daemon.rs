use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use rusqlite::Connection;

use crate::api::haiku::HaikuClient;
use crate::consolidate::{consolidation_loop, decay};
use crate::context::generator;
use crate::db::{consolidations, fts, memories};
use crate::ingest::pipeline;
use crate::ipc::protocol::*;

/// Shared daemon state, accessible from IPC handlers and background tasks.
pub struct DaemonState {
    pub db: Mutex<Connection>,
    pub global_db: Option<Mutex<Connection>>,
    pub api: HaikuClient,
    pub last_consolidation: Mutex<Option<String>>,
    pub consolidation_interval_secs: AtomicU64,
}

impl DaemonState {
    pub fn new(db: Connection, global_db: Option<Connection>, api: HaikuClient, consolidation_interval: u64) -> Self {
        Self {
            db: Mutex::new(db),
            global_db: global_db.map(Mutex::new),
            api,
            last_consolidation: Mutex::new(None),
            consolidation_interval_secs: AtomicU64::new(consolidation_interval),
        }
    }

    pub async fn handle_ingest(&self, params: IngestParams) -> Response {
        // Phase 1: API call (async, no DB lock)
        let extraction = pipeline::extract(&self.api, &params.content).await;

        // Check if this should go to global DB
        let is_global = extraction
            .as_ref()
            .map(|e| e.is_global.unwrap_or(false))
            .unwrap_or(false);

        // Phase 2: DB storage (sync, short lock)
        let conn = self.db.lock().unwrap();
        match pipeline::store(&conn, &params.content, extraction, params.session_id.as_deref()) {
            Ok(result) => {
                // Also store in global DB if flagged global
                if is_global {
                    if let Some(ref global_db) = self.global_db {
                        let gconn = global_db.lock().unwrap();
                        // Re-store without extraction (already stored in project DB, just copy)
                        let _ = pipeline::store(&gconn, &params.content, None, params.session_id.as_deref());
                    }
                }
                Response::ok(IngestResult {
                    memory_id: result.memory_id,
                    deduplicated: result.deduplicated,
                })
            }
            Err(e) => Response::error(e),
        }
    }

    pub async fn handle_get_context(&self, params: GetContextParams) -> Response {
        // Phase 1: Fetch data from DB (sync, short lock)
        let (all_memories, insights) = {
            let conn = self.db.lock().unwrap();
            let mems = memories::get_by_importance(&conn, 100).unwrap_or_default();
            let ins = consolidations::get_recent(&conn, 20).unwrap_or_default();
            (mems, ins)
        };
        // Lock dropped here

        // Phase 2: Build context, possibly calling Haiku (async, no DB lock)
        let context =
            generator::build_context(all_memories, insights, &self.api, params.max_tokens).await;
        let token_estimate = context.len() / 4;

        // Phase 3: Record session context (sync, short lock)
        if let Some(ref session_id) = params.session_id {
            let conn = self.db.lock().unwrap();
            let _ = conn.execute(
                "INSERT INTO session_context (session_id, memory_ids) VALUES (?1, '[]')",
                rusqlite::params![session_id],
            );
        }

        Response::ok(ContextData {
            context,
            token_estimate,
        })
    }

    pub fn handle_get_status(&self) -> Response {
        let conn = self.db.lock().unwrap();
        let total_memories = memories::total_count(&conn).unwrap_or(0);
        let total_consolidations = consolidations::total_count(&conn).unwrap_or(0);
        let memories_by_type = memories::count_by_type(&conn).unwrap_or_default();
        let last_consolidation = self.last_consolidation.lock().unwrap().clone();

        let consolidation_interval_secs = self.consolidation_interval_secs.load(Ordering::Relaxed);

        Response::ok(serde_json::json!({
            "total_memories": total_memories,
            "total_consolidations": total_consolidations,
            "memories_by_type": memories_by_type,
            "last_consolidation": last_consolidation,
            "consolidation_interval_secs": consolidation_interval_secs,
        }))
    }

    pub fn handle_end_session(&self, params: EndSessionParams) -> Response {
        let conn = self.db.lock().unwrap();
        let cleaned = decay::cleanup_expired(&conn).unwrap_or(0);

        Response::ok(serde_json::json!({
            "session_id": params.session_id,
            "expired_cleaned": cleaned,
        }))
    }

    pub fn handle_search(&self, params: SearchParams) -> Response {
        let conn = self.db.lock().unwrap();
        let limit = params.limit.unwrap_or(10);
        match fts::search(&conn, &params.query, limit) {
            Ok(ids) => Response::ok(ids),
            Err(e) => Response::error(format!("search failed: {e}")),
        }
    }

    /// Process a session-end summary by extracting multiple memories from it via Haiku.
    pub async fn handle_session_end_summary(
        &self,
        summary: &str,
        session_id: Option<&str>,
    ) -> Response {
        // First, clean up expired memories
        {
            let conn = self.db.lock().unwrap();
            let _ = decay::cleanup_expired(&conn);
        }

        // Ask Haiku to extract individual memories from the session summary
        if !self.api.is_available() {
            // No API: store the whole summary as a single memory
            let conn = self.db.lock().unwrap();
            match pipeline::store(&conn, summary, None, session_id) {
                Ok(result) => {
                    return Response::ok(serde_json::json!({
                        "memories_extracted": 1,
                        "memory_ids": [result.memory_id],
                    }));
                }
                Err(e) => return Response::error(e),
            }
        }

        let system = "You are a memory extraction system. Extract individual, distinct memories from a session summary. Return JSON only, no markdown fences.";
        let user_msg = format!(
            "Extract individual memories from this coding session summary:\n\n\"{summary}\"\n\n\
             Return a JSON array of strings, where each string is a single distinct observation, \
             decision, pattern, or fact worth remembering. Focus on:\n\
             - Architectural decisions made\n\
             - Patterns discovered or established\n\
             - Gotchas encountered\n\
             - Preferences expressed\n\
             - Key progress milestones\n\n\
             Return: [\"memory 1\", \"memory 2\", ...]"
        );

        let memories_to_store: Vec<String> = match self.api.complete(system, &user_msg).await {
            Ok(text) => {
                let cleaned = text
                    .trim()
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();

                serde_json::from_str(cleaned).unwrap_or_else(|_| vec![summary.to_string()])
            }
            Err(_) => vec![summary.to_string()],
        };

        let mut memory_ids = Vec::new();
        for note in &memories_to_store {
            // Extract with Haiku (async, no DB lock)
            let extraction = pipeline::extract(&self.api, note).await;

            // Store in DB (sync, short lock)
            let conn = self.db.lock().unwrap();
            match pipeline::store(&conn, note, extraction, session_id) {
                Ok(result) => memory_ids.push(result.memory_id),
                Err(e) => {
                    tracing::warn!("Failed to store extracted memory: {e}");
                }
            }
        }

        Response::ok(serde_json::json!({
            "memories_extracted": memory_ids.len(),
            "memory_ids": memory_ids,
        }))
    }

    /// Manually trigger a consolidation pass.
    pub async fn handle_consolidate(&self) -> Response {
        // Phase 1: Fetch unconsolidated memories (sync, short lock)
        let data = {
            let conn = self.db.lock().unwrap();
            consolidation_loop::fetch_unconsolidated(&conn)
        };

        let Some((unconsolidated, recent_insights)) = data else {
            return Response::ok(serde_json::json!({
                "consolidated": 0,
                "message": "Not enough unconsolidated memories (need at least 3)",
            }));
        };

        let count = unconsolidated.len();
        let ids: Vec<i64> = unconsolidated.iter().map(|m| m.id).collect();

        // Phase 2: Haiku analysis (async, no lock)
        let result = consolidation_loop::analyze(&self.api, &unconsolidated, &recent_insights).await;

        match result {
            Some(result) => {
                // Phase 3: Apply results (sync, short lock)
                let conn = self.db.lock().unwrap();
                consolidation_loop::apply(&conn, result, &ids);
                self.update_last_consolidation();

                Response::ok(serde_json::json!({
                    "consolidated": count,
                    "message": format!("Consolidated {count} memories"),
                }))
            }
            None => Response::ok(serde_json::json!({
                "consolidated": 0,
                "message": "Consolidation analysis failed (Haiku unavailable or error)",
            })),
        }
    }

    /// Update runtime configuration.
    pub fn handle_configure(&self, consolidation_interval: Option<u64>) -> Response {
        let mut changed = Vec::new();

        if let Some(secs) = consolidation_interval {
            self.consolidation_interval_secs.store(secs, Ordering::Relaxed);
            changed.push(format!("consolidation_interval: {secs}s"));
        }

        if changed.is_empty() {
            return Response::error("No settings provided to update");
        }

        Response::ok(serde_json::json!({
            "updated": changed,
            "consolidation_interval_secs": self.consolidation_interval_secs.load(Ordering::Relaxed),
        }))
    }

    /// Adjust a memory's importance based on recall feedback.
    pub fn handle_feedback(&self, memory_id: i64, helpful: bool) -> Response {
        let delta = if helpful { 0.1 } else { -0.1 };
        let conn = self.db.lock().unwrap();
        match memories::update_importance(&conn, memory_id, delta) {
            Ok(()) => Response::ok(serde_json::json!({
                "memory_id": memory_id,
                "delta": delta,
            })),
            Err(e) => Response::error(format!("Failed to update importance: {e}")),
        }
    }

    /// Delete a memory by ID.
    pub fn handle_delete(&self, memory_id: i64) -> Response {
        let conn = self.db.lock().unwrap();
        match memories::delete_by_id(&conn, memory_id) {
            Ok(()) => Response::ok(serde_json::json!({
                "deleted": memory_id,
            })),
            Err(e) => Response::error(format!("Failed to delete memory #{memory_id}: {e}")),
        }
    }

    /// List all memories (no search required).
    pub fn handle_list(&self, limit: usize, memory_type: Option<&str>) -> Response {
        let conn = self.db.lock().unwrap();
        let all = memories::get_all(&conn, limit).unwrap_or_default();
        let filtered: Vec<&memories::Memory> = if let Some(mtype) = memory_type {
            all.iter().filter(|m| m.memory_type == mtype).collect()
        } else {
            all.iter().collect()
        };

        let lines: Vec<String> = filtered
            .iter()
            .map(|m| {
                format!(
                    "[#{id}] [{mtype}] (importance: {imp:.1}) {summary}",
                    id = m.id,
                    mtype = m.memory_type,
                    imp = m.importance,
                    summary = m.summary.as_deref().unwrap_or(&m.content),
                )
            })
            .collect();

        Response::ok(serde_json::json!({
            "count": lines.len(),
            "memories": lines.join("\n"),
        }))
    }

    /// Update an existing memory's content.
    pub async fn handle_update(&self, memory_id: i64, content: &str) -> Response {
        // Re-extract with Haiku for new summary
        let summary = if self.api.is_available() {
            let system = "You are a memory processor. Summarize this observation in one line, max 20 words. Return only the summary text, nothing else.";
            match self.api.complete(system, content).await {
                Ok(text) => Some(text.trim().to_string()),
                Err(_) => None,
            }
        } else {
            None
        };

        let conn = self.db.lock().unwrap();
        match memories::update_content(&conn, memory_id, content, summary.as_deref()) {
            Ok(()) => Response::ok(serde_json::json!({
                "memory_id": memory_id,
                "updated": true,
            })),
            Err(e) => Response::error(format!("Failed to update memory #{memory_id}: {e}")),
        }
    }

    /// Export all memories as JSON.
    pub fn handle_export(&self) -> Response {
        let conn = self.db.lock().unwrap();
        match memories::export_all(&conn) {
            Ok(all) => Response::ok(serde_json::json!({
                "count": all.len(),
                "memories": all,
            })),
            Err(e) => Response::error(format!("Export failed: {e}")),
        }
    }

    /// Import memories from JSON array.
    pub async fn handle_import(&self, items: &[serde_json::Value]) -> Response {
        let mut imported = 0;
        for item in items {
            let content = match item.get("content").and_then(|v| v.as_str()) {
                Some(c) => c,
                None => continue,
            };

            let extraction = pipeline::extract(&self.api, content).await;
            let conn = self.db.lock().unwrap();
            if pipeline::store(&conn, content, extraction, None).is_ok() {
                imported += 1;
            }
        }

        Response::ok(serde_json::json!({
            "imported": imported,
        }))
    }

    /// Sync global memories: copy is_global memories to/from the global DB.
    pub fn sync_global_memories(&self) {
        let Some(ref global_db) = self.global_db else { return };

        let global_conn = global_db.lock().unwrap();
        let project_conn = self.db.lock().unwrap();

        // Export is_global memories from project → global
        if let Ok(project_globals) = memories::get_global(&project_conn, 100) {
            for mem in &project_globals {
                // Check if already in global DB by content similarity
                let exists: bool = global_conn
                    .query_row(
                        "SELECT count(*) > 0 FROM memories WHERE summary = ?1",
                        rusqlite::params![mem.summary],
                        |row| row.get(0),
                    )
                    .unwrap_or(true);

                if !exists {
                    let new_mem = memories::NewMemory {
                        content: mem.content.clone(),
                        summary: mem.summary.clone(),
                        entities: mem.entities.as_ref().and_then(|e| serde_json::from_str(e).ok()),
                        topics: mem.topics.as_ref().and_then(|t| serde_json::from_str(t).ok()),
                        semantic_tags: mem.semantic_tags.as_ref().and_then(|t| serde_json::from_str(t).ok()),
                        memory_type: mem.memory_type.clone(),
                        importance: mem.importance,
                        source_session: mem.source_session.clone(),
                        decay_at: mem.decay_at.clone(),
                        is_global: true,
                    };
                    let _ = memories::insert(&global_conn, &new_mem);
                }
            }
        }

        // Import global memories → project (if not already present)
        if let Ok(all_global) = memories::get_all(&global_conn, 100) {
            for mem in &all_global {
                let exists: bool = project_conn
                    .query_row(
                        "SELECT count(*) > 0 FROM memories WHERE summary = ?1 AND is_global = 1",
                        rusqlite::params![mem.summary],
                        |row| row.get(0),
                    )
                    .unwrap_or(true);

                if !exists {
                    let new_mem = memories::NewMemory {
                        content: mem.content.clone(),
                        summary: mem.summary.clone(),
                        entities: mem.entities.as_ref().and_then(|e| serde_json::from_str(e).ok()),
                        topics: mem.topics.as_ref().and_then(|t| serde_json::from_str(t).ok()),
                        semantic_tags: mem.semantic_tags.as_ref().and_then(|t| serde_json::from_str(t).ok()),
                        memory_type: mem.memory_type.clone(),
                        importance: mem.importance,
                        source_session: mem.source_session.clone(),
                        decay_at: mem.decay_at.clone(),
                        is_global: true,
                    };
                    let _ = memories::insert(&project_conn, &new_mem);
                }
            }
        }
    }

    /// Generate CLAUDE.md snippet for memory tool usage.
    pub fn handle_setup(&self) -> Response {
        let snippet = r#"# Memory: claude-remember MCP

Use the `claude-remember` MCP tools every session:
- **Start**: Call `memory_context` to load project memory before doing work
- **During**: Call `memory_remember` to store decisions, patterns, gotchas, architecture
- **Search**: Call `memory_recall` to find relevant memories by keyword/topic
- **Rate**: Call `memory_feedback` after recall to mark memories helpful (true) or not (false)
- **End**: Call `memory_session_end` with a summary of what was accomplished
- **Check**: Call `memory_status` to view memory system health and stats"#;

        Response::ok(serde_json::json!({
            "snippet": snippet,
            "instructions": "Add this to the top of your project's CLAUDE.md file to enable automatic memory usage.",
        }))
    }

    pub fn update_last_consolidation(&self) {
        let now = chrono_now();
        *self.last_consolidation.lock().unwrap() = Some(now);
    }
}

fn chrono_now() -> String {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.query_row("SELECT datetime('now')", [], |row| row.get(0))
        .unwrap_or_else(|_| "unknown".to_string())
}
