use std::sync::Mutex;

use rusqlite::Connection;

use crate::api::haiku::HaikuClient;
use crate::consolidate::decay;
use crate::context::generator;
use crate::db::{consolidations, fts, memories};
use crate::ingest::pipeline;
use crate::ipc::protocol::*;

/// Shared daemon state, accessible from IPC handlers and background tasks.
pub struct DaemonState {
    pub db: Mutex<Connection>,
    pub api: HaikuClient,
    pub last_consolidation: Mutex<Option<String>>,
}

impl DaemonState {
    pub fn new(db: Connection, api: HaikuClient) -> Self {
        Self {
            db: Mutex::new(db),
            api,
            last_consolidation: Mutex::new(None),
        }
    }

    pub async fn handle_ingest(&self, params: IngestParams) -> Response {
        // Phase 1: API call (async, no DB lock)
        let extraction = pipeline::extract(&self.api, &params.content).await;

        // Phase 2: DB storage (sync, short lock)
        let conn = self.db.lock().unwrap();
        match pipeline::store(&conn, &params.content, extraction, params.session_id.as_deref()) {
            Ok(result) => Response::ok(IngestResult {
                memory_id: result.memory_id,
                deduplicated: result.deduplicated,
            }),
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

        Response::ok(StatusData {
            total_memories,
            total_consolidations,
            memories_by_type,
            last_consolidation,
        })
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
