use rusqlite::Connection;
use serde::Deserialize;
use tracing::{info, warn};

use crate::api::haiku::HaikuClient;
use crate::db::memories::Memory;
use crate::db::consolidations::Consolidation;
use crate::db::{consolidations, memories};

#[derive(Debug, Deserialize)]
pub(crate) struct ConsolidationResult {
    connections: Option<Vec<(i64, i64)>>,
    insights: Option<Vec<InsightEntry>>,
    merge_candidates: Option<Vec<Vec<i64>>>,
    obsolete: Option<Vec<i64>>,
}

#[derive(Debug, Deserialize)]
struct InsightEntry {
    insight: String,
    memory_ids: Vec<i64>,
    topics: Option<Vec<String>>,
}

/// Fetch unconsolidated data from DB (sync).
pub fn fetch_unconsolidated(conn: &Connection) -> Option<(Vec<Memory>, Vec<Consolidation>)> {
    let unconsolidated = memories::get_unconsolidated(conn, 50).ok()?;
    if unconsolidated.len() < 3 {
        return None;
    }
    let recent_insights = consolidations::get_recent(conn, 10).unwrap_or_default();
    Some((unconsolidated, recent_insights))
}

/// Call Haiku for consolidation analysis (async, no DB).
pub async fn analyze(
    api: &HaikuClient,
    unconsolidated: &[Memory],
    recent_insights: &[Consolidation],
) -> Option<ConsolidationResult> {
    if !api.is_available() {
        return None;
    }

    let memories_text: String = unconsolidated
        .iter()
        .map(|m| {
            format!(
                "ID:{} [{}] (importance:{:.1}) {}",
                m.id,
                m.memory_type,
                m.importance,
                m.summary.as_deref().unwrap_or(&m.content)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let insights_text: String = recent_insights
        .iter()
        .map(|i| format!("- {}", i.insight))
        .collect::<Vec<_>>()
        .join("\n");

    let system = "You are a memory consolidation system. You find connections between memories, generate cross-cutting insights, and identify memories that can be merged or compressed. Return JSON only, no markdown fences.";

    let user_msg = format!(
        "Here are recent unconsolidated memories:\n{memories_text}\n\n\
         Here are existing insights:\n{insights_text}\n\n\
         Find:\n\
         1. connections: pairs of memory IDs [id1, id2] that relate to each other\n\
         2. insights: cross-cutting observations (max 3), each with {{\"insight\": \"...\", \"memory_ids\": [...], \"topics\": [...]}}\n\
         3. merge_candidates: groups of memory IDs saying the same thing differently\n\
         4. obsolete: memory IDs superseded by newer information\n\n\
         Return: {{\"connections\": [...], \"insights\": [...], \"merge_candidates\": [...], \"obsolete\": [...]}}"
    );

    let response_text = match api.complete(system, &user_msg).await {
        Ok(text) => text,
        Err(e) => {
            warn!("Consolidation API call failed: {e}");
            return None;
        }
    };

    let cleaned = response_text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    match serde_json::from_str::<ConsolidationResult>(cleaned) {
        Ok(r) => Some(r),
        Err(e) => {
            warn!("Failed to parse consolidation result: {e}. Raw: {cleaned}");
            None
        }
    }
}

/// Apply consolidation results to DB (sync).
pub fn apply(conn: &Connection, result: ConsolidationResult, memory_ids: &[i64]) {
    if let Some(insights) = result.insights {
        for entry in insights {
            if let Err(e) = consolidations::insert(
                conn,
                &entry.memory_ids,
                &entry.insight,
                entry.topics.as_deref(),
            ) {
                warn!("Failed to insert consolidation: {e}");
            }
        }
    }

    if let Some(merge_groups) = result.merge_candidates {
        for group in merge_groups {
            if group.len() < 2 {
                continue;
            }
            for id in &group[..group.len() - 1] {
                let _ = memories::delete_by_id(conn, *id);
            }
            info!("Merged {} memories, kept #{}", group.len(), group.last().unwrap());
        }
    }

    if let Some(obsolete_ids) = result.obsolete {
        for id in obsolete_ids {
            let _ = memories::delete_by_id(conn, id);
            info!("Removed obsolete memory #{id}");
        }
    }

    if let Err(e) = memories::mark_consolidated(conn, memory_ids) {
        warn!("Failed to mark memories as consolidated: {e}");
    }

    info!("Consolidation pass complete: processed {} memories", memory_ids.len());
}
