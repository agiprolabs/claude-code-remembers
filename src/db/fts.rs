use rusqlite::{params, Connection};

/// Search memories using FTS5 full-text search.
/// Returns memory IDs ranked by relevance.
pub fn search(conn: &Connection, query: &str, limit: usize) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT rowid FROM memory_fts WHERE memory_fts MATCH ?1 ORDER BY rank LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![query, limit as i64], |row| row.get(0))?;
    rows.collect()
}

/// Search and return summaries for dedup comparison.
pub fn search_summaries(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.summary FROM memories m
         JOIN memory_fts f ON m.id = f.rowid
         WHERE memory_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![query, limit as i64], |row| {
        let summary: Option<String> = row.get(1)?;
        Ok((row.get(0)?, summary.unwrap_or_default()))
    })?;

    rows.collect()
}
