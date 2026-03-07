use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub content: String,
    pub summary: Option<String>,
    pub entities: Option<String>,
    pub topics: Option<String>,
    pub semantic_tags: Option<String>,
    pub memory_type: String,
    pub importance: f64,
    pub source_session: Option<String>,
    pub consolidated: bool,
    pub decay_at: Option<String>,
    pub is_global: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NewMemory {
    pub content: String,
    pub summary: Option<String>,
    pub entities: Option<Vec<String>>,
    pub topics: Option<Vec<String>>,
    pub semantic_tags: Option<Vec<String>>,
    pub memory_type: String,
    pub importance: f64,
    pub source_session: Option<String>,
    pub decay_at: Option<String>,
    pub is_global: bool,
}

impl NewMemory {
    /// Compute decay_at based on memory_type if not explicitly set.
    pub fn with_default_decay(mut self) -> Self {
        if self.decay_at.is_none() {
            self.decay_at = match self.memory_type.as_str() {
                "progress" => Some(days_from_now(7)),
                "preference" | "gotcha" => Some(days_from_now(90)),
                _ => None, // architecture, decision, pattern are permanent
            };
        }
        self
    }
}

fn days_from_now(days: i64) -> String {
    // SQLite datetime arithmetic
    format!("datetime('now', '+{days} days')")
}

pub fn insert(conn: &Connection, mem: &NewMemory) -> rusqlite::Result<i64> {
    let entities_json = mem.entities.as_ref().map(|e| serde_json::to_string(e).unwrap());
    let topics_json = mem.topics.as_ref().map(|t| serde_json::to_string(t).unwrap());
    let tags_json = mem.semantic_tags.as_ref().map(|t| serde_json::to_string(t).unwrap());

    // Handle decay_at: if it's a SQL expression, use it directly; otherwise treat as literal
    let decay_expr = mem.decay_at.as_deref();
    let is_sql_expr = decay_expr
        .map(|d| d.starts_with("datetime("))
        .unwrap_or(false);

    if is_sql_expr {
        let sql = format!(
            "INSERT INTO memories (content, summary, entities, topics, semantic_tags, memory_type, importance, source_session, decay_at, is_global)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, {}, ?9)",
            decay_expr.unwrap()
        );
        conn.execute(
            &sql,
            params![
                mem.content,
                mem.summary,
                entities_json,
                topics_json,
                tags_json,
                mem.memory_type,
                mem.importance,
                mem.source_session,
                mem.is_global,
            ],
        )?;
    } else {
        conn.execute(
            "INSERT INTO memories (content, summary, entities, topics, semantic_tags, memory_type, importance, source_session, decay_at, is_global)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                mem.content,
                mem.summary,
                entities_json,
                topics_json,
                tags_json,
                mem.memory_type,
                mem.importance,
                mem.source_session,
                decay_expr,
                mem.is_global,
            ],
        )?;
    }

    Ok(conn.last_insert_rowid())
}

pub fn update_importance(conn: &Connection, id: i64, delta: f64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE memories SET importance = MIN(1.0, MAX(0.0, importance + ?1)), updated_at = datetime('now') WHERE id = ?2",
        params![delta, id],
    )?;
    Ok(())
}

pub fn get_by_importance(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, summary, entities, topics, semantic_tags, memory_type, importance,
                source_session, consolidated, decay_at, is_global, created_at, updated_at
         FROM memories
         WHERE decay_at IS NULL OR decay_at > datetime('now')
         ORDER BY importance DESC, updated_at DESC
         LIMIT ?1",
    )?;

    let rows = stmt.query_map(params![limit as i64], row_to_memory)?;
    rows.collect()
}

pub fn get_unconsolidated(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, summary, entities, topics, semantic_tags, memory_type, importance,
                source_session, consolidated, decay_at, is_global, created_at, updated_at
         FROM memories
         WHERE consolidated = 0
         ORDER BY created_at DESC
         LIMIT ?1",
    )?;

    let rows = stmt.query_map(params![limit as i64], row_to_memory)?;
    rows.collect()
}

pub fn mark_consolidated(conn: &Connection, ids: &[i64]) -> rusqlite::Result<()> {
    for id in ids {
        conn.execute(
            "UPDATE memories SET consolidated = 1, updated_at = datetime('now') WHERE id = ?1",
            params![id],
        )?;
    }
    Ok(())
}

pub fn delete_by_id(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn delete_expired(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM memories WHERE decay_at IS NOT NULL AND decay_at <= datetime('now')",
        [],
    )
}

pub fn count_by_type(conn: &Connection) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT memory_type, count(*) FROM memories
         WHERE decay_at IS NULL OR decay_at > datetime('now')
         GROUP BY memory_type",
    )?;

    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

pub fn total_count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT count(*) FROM memories WHERE decay_at IS NULL OR decay_at > datetime('now')",
        [],
        |row| row.get(0),
    )
}

pub fn get_all(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, summary, entities, topics, semantic_tags, memory_type, importance,
                source_session, consolidated, decay_at, is_global, created_at, updated_at
         FROM memories
         WHERE decay_at IS NULL OR decay_at > datetime('now')
         ORDER BY importance DESC, updated_at DESC
         LIMIT ?1",
    )?;

    let rows = stmt.query_map(params![limit as i64], row_to_memory)?;
    rows.collect()
}

pub fn get_by_id(conn: &Connection, id: i64) -> rusqlite::Result<Option<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, summary, entities, topics, semantic_tags, memory_type, importance,
                source_session, consolidated, decay_at, is_global, created_at, updated_at
         FROM memories WHERE id = ?1",
    )?;

    let mut rows = stmt.query_map(params![id], row_to_memory)?;
    match rows.next() {
        Some(Ok(m)) => Ok(Some(m)),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

pub fn update_content(conn: &Connection, id: i64, content: &str, summary: Option<&str>) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE memories SET content = ?1, summary = ?2, updated_at = datetime('now') WHERE id = ?3",
        params![content, summary, id],
    )?;
    Ok(())
}

pub fn get_global(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, summary, entities, topics, semantic_tags, memory_type, importance,
                source_session, consolidated, decay_at, is_global, created_at, updated_at
         FROM memories
         WHERE is_global = 1 AND (decay_at IS NULL OR decay_at > datetime('now'))
         ORDER BY importance DESC
         LIMIT ?1",
    )?;

    let rows = stmt.query_map(params![limit as i64], row_to_memory)?;
    rows.collect()
}

pub fn export_all(conn: &Connection) -> rusqlite::Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, summary, entities, topics, semantic_tags, memory_type, importance,
                source_session, consolidated, decay_at, is_global, created_at, updated_at
         FROM memories ORDER BY id",
    )?;

    let rows = stmt.query_map([], row_to_memory)?;
    rows.collect()
}

fn row_to_memory(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get(0)?,
        content: row.get(1)?,
        summary: row.get(2)?,
        entities: row.get(3)?,
        topics: row.get(4)?,
        semantic_tags: row.get(5)?,
        memory_type: row.get(6)?,
        importance: row.get(7)?,
        source_session: row.get(8)?,
        consolidated: row.get(9)?,
        decay_at: row.get(10)?,
        is_global: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}
