use tracing::warn;

use crate::api::haiku::HaikuClient;
use crate::db::consolidations::Consolidation;
use crate::db::memories::Memory;

/// Build context from pre-fetched data (sync part — no DB access here).
fn build_context_from_data(all_memories: &[Memory], insights: &[Consolidation]) -> String {
    if all_memories.is_empty() && insights.is_empty() {
        return String::from("# Project Memory\n\nNo memories recorded yet.");
    }

    let mut architecture: Vec<&str> = Vec::new();
    let mut decisions: Vec<&str> = Vec::new();
    let mut patterns: Vec<&str> = Vec::new();
    let mut gotchas: Vec<&str> = Vec::new();
    let mut preferences: Vec<&str> = Vec::new();
    let mut progress: Vec<&str> = Vec::new();

    for m in all_memories {
        let display = m
            .summary
            .as_deref()
            .unwrap_or_else(|| truncate(&m.content, 80));

        match m.memory_type.as_str() {
            "architecture" => architecture.push(display),
            "decision" => decisions.push(display),
            "pattern" => patterns.push(display),
            "gotcha" => gotchas.push(display),
            "preference" => preferences.push(display),
            _ => progress.push(display),
        }
    }

    let mut output = String::from("# Project Memory\n\n");

    let sections: &[(&str, &[&str])] = &[
        ("Architecture", &architecture),
        ("Decisions", &decisions),
        ("Patterns", &patterns),
        ("Gotchas", &gotchas),
        ("Preferences", &preferences),
        ("Recent Progress", &progress),
    ];

    for (heading, items) in sections {
        if items.is_empty() {
            continue;
        }
        output.push_str(&format!("## {heading}\n"));
        for item in items.iter().take(10) {
            output.push_str(&format!("- {item}\n"));
        }
        output.push('\n');
    }

    if !insights.is_empty() {
        output.push_str("## Key Insights\n");
        for insight in insights.iter().take(5) {
            output.push_str(&format!("- {}\n", insight.insight));
        }
        output.push('\n');
    }

    output
}

/// Build context, compressing with Haiku if over budget.
/// Takes pre-fetched data to avoid holding DB lock across await.
pub async fn build_context(
    all_memories: Vec<Memory>,
    insights: Vec<Consolidation>,
    api: &HaikuClient,
    max_tokens: usize,
) -> String {
    let mut output = build_context_from_data(&all_memories, &insights);

    // Token budget check (rough: 1 token ≈ 4 chars)
    let estimated_tokens = output.len() / 4;
    if estimated_tokens > max_tokens && api.is_available() {
        match compress_with_haiku(api, &output, max_tokens).await {
            Ok(compressed) => return compressed,
            Err(e) => warn!("Compression failed, using uncompressed: {e}"),
        }
    }

    // If still over budget without Haiku, truncate
    if output.len() / 4 > max_tokens {
        let char_budget = max_tokens * 4;
        if output.len() > char_budget {
            output.truncate(char_budget);
            if let Some(pos) = output.rfind('\n') {
                output.truncate(pos + 1);
            }
            output.push_str("\n(truncated to fit token budget)\n");
        }
    }

    output
}

async fn compress_with_haiku(
    api: &HaikuClient,
    content: &str,
    max_tokens: usize,
) -> Result<String, String> {
    let system = "You are a text compressor. Compress the given project memory summary to fit within the specified token budget while preserving the most important information. Keep the markdown structure. Return only the compressed text, no explanation.";

    let user_msg = format!(
        "Compress this to fit within {max_tokens} tokens (approximately {} characters):\n\n{content}",
        max_tokens * 4
    );

    api.complete(system, &user_msg)
        .await
        .map_err(|e| format!("compression API call failed: {e}"))
}

fn truncate(s: &str, max_chars: usize) -> &str {
    if s.len() <= max_chars {
        s
    } else {
        let mut end = max_chars;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}
