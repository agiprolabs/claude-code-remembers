use rusqlite::Connection;
use serde::Deserialize;
use tracing::{info, warn};

use crate::api::haiku::HaikuClient;
use crate::db::{fts, memories};
use crate::ingest::dedup;

#[derive(Debug, Deserialize)]
pub(crate) struct HaikuExtraction {
    pub(crate) summary: Option<String>,
    pub(crate) entities: Option<Vec<String>>,
    pub(crate) topics: Option<Vec<String>>,
    pub(crate) semantic_tags: Option<Vec<String>>,
    pub(crate) memory_type: Option<String>,
    pub(crate) importance: Option<f64>,
    pub(crate) is_duplicate_of: Option<String>,
    pub(crate) is_global: Option<bool>,
}

pub struct IngestResult {
    pub memory_id: i64,
    pub deduplicated: bool,
}

/// Extract structured data from a raw note via Haiku (async, no DB).
async fn extract_with_haiku(api: &HaikuClient, raw_note: &str) -> Option<HaikuExtraction> {
    let system = "You are a memory processor. Extract structured information from coding session observations. Return JSON only, no markdown fences, no explanation.";

    let user_msg = format!(
        "Process this observation from a coding session:\n\n\"{raw_note}\"\n\n\
         Return a JSON object with these fields:\n\
         - \"summary\": one line, max 20 words\n\
         - \"entities\": list of proper nouns and key technical terms\n\
         - \"topics\": category tags\n\
         - \"semantic_tags\": 5-10 semantic keywords/synonyms for search (include related concepts, alternate phrasings, and broader/narrower terms)\n\
         - \"memory_type\": one of \"architecture\", \"decision\", \"pattern\", \"gotcha\", \"preference\", \"progress\"\n\
         - \"importance\": 0.0 to 1.0\n\
         - \"is_duplicate_of\": null, or a summary of an existing memory this duplicates\n\
         - \"is_global\": true if this is universal knowledge (user preferences, tool patterns) not specific to one project, false otherwise"
    );

    match api.complete(system, &user_msg).await {
        Ok(text) => {
            let cleaned = text
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();

            match serde_json::from_str::<HaikuExtraction>(cleaned) {
                Ok(ext) => Some(ext),
                Err(e) => {
                    warn!("Failed to parse Haiku extraction: {e}. Raw: {cleaned}");
                    None
                }
            }
        }
        Err(e) => {
            warn!("Haiku extraction failed: {e}");
            None
        }
    }
}

/// Run the Haiku extraction (async), then store in DB (sync).
/// Split into two phases so MutexGuard is never held across .await.
pub async fn extract(api: &HaikuClient, raw_note: &str) -> Option<HaikuExtraction> {
    if api.is_available() {
        extract_with_haiku(api, raw_note).await
    } else {
        None
    }
}

/// Store a processed extraction into the database (sync, no .await).
pub fn store(
    conn: &Connection,
    raw_note: &str,
    extraction: Option<HaikuExtraction>,
    session_id: Option<&str>,
) -> Result<IngestResult, String> {
    let (summary, entities, topics, semantic_tags, memory_type, importance, is_dup, is_global) =
        match extraction {
            Some(ext) => (
                ext.summary,
                ext.entities,
                ext.topics,
                ext.semantic_tags,
                ext.memory_type.unwrap_or_else(|| "progress".to_string()),
                ext.importance.unwrap_or(0.5),
                ext.is_duplicate_of,
                ext.is_global.unwrap_or(false),
            ),
            None => (
                None,
                None,
                None,
                None,
                "progress".to_string(),
                0.5,
                None,
                false,
            ),
        };

    // Check for duplicates via FTS + Jaccard
    let mut deduplicated = false;
    if let Some(ref summary_text) = summary {
        if let Ok(candidates) = fts::search_summaries(conn, summary_text, 5) {
            for (existing_id, existing_summary) in &candidates {
                let sim = dedup::jaccard_similarity(summary_text, existing_summary);
                if sim > dedup::DEDUP_THRESHOLD {
                    info!(
                        "Dedup: new memory similar to #{existing_id} (similarity: {sim:.2}), replacing"
                    );
                    let _ = memories::delete_by_id(conn, *existing_id);
                    deduplicated = true;
                    break;
                }
            }
        }
    }

    // Also check if Haiku flagged it as a duplicate
    if is_dup.is_some() && !deduplicated {
        if let Some(ref dup_summary) = is_dup {
            if let Ok(candidates) = fts::search_summaries(conn, dup_summary, 3) {
                for (existing_id, existing_summary) in &candidates {
                    let sim = dedup::jaccard_similarity(dup_summary, existing_summary);
                    if sim > 0.4 {
                        let _ = memories::delete_by_id(conn, *existing_id);
                        deduplicated = true;
                        break;
                    }
                }
            }
        }
    }

    let new_mem = memories::NewMemory {
        content: raw_note.to_string(),
        summary,
        entities,
        topics,
        semantic_tags,
        memory_type,
        importance,
        source_session: session_id.map(|s| s.to_string()),
        decay_at: None,
        is_global,
    }
    .with_default_decay();

    let id = memories::insert(conn, &new_mem).map_err(|e| format!("db insert error: {e}"))?;

    Ok(IngestResult {
        memory_id: id,
        deduplicated,
    })
}
