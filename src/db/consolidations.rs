use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consolidation {
    pub id: i64,
    pub memory_ids: Vec<i64>,
    pub insight: String,
    pub topics: Option<Vec<String>>,
    pub created_at: String,
}

pub fn insert(
    conn: &Connection,
    memory_ids: &[i64],
    insight: &str,
    topics: Option<&[String]>,
) -> rusqlite::Result<i64> {
    let ids_json = serde_json::to_string(memory_ids).unwrap();
    let topics_json = topics.map(|t| serde_json::to_string(t).unwrap());

    conn.execute(
        "INSERT INTO consolidations (memory_ids, insight, topics) VALUES (?1, ?2, ?3)",
        params![ids_json, insight, topics_json],
    )?;

    Ok(conn.last_insert_rowid())
}

pub fn get_recent(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<Consolidation>> {
    let mut stmt = conn.prepare(
        "SELECT id, memory_ids, insight, topics, created_at
         FROM consolidations
         ORDER BY created_at DESC
         LIMIT ?1",
    )?;

    let rows = stmt.query_map(params![limit as i64], |row| {
        let ids_str: String = row.get(1)?;
        let topics_str: Option<String> = row.get(3)?;

        Ok(Consolidation {
            id: row.get(0)?,
            memory_ids: serde_json::from_str(&ids_str).unwrap_or_default(),
            insight: row.get(2)?,
            topics: topics_str.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: row.get(4)?,
        })
    })?;

    rows.collect()
}

pub fn total_count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT count(*) FROM consolidations", [], |row| row.get(0))
}
