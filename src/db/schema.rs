use rusqlite::Connection;

pub fn initialize(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS memories (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            content        TEXT NOT NULL,
            summary        TEXT,
            entities       TEXT,
            topics         TEXT,
            semantic_tags  TEXT,
            memory_type    TEXT NOT NULL DEFAULT 'progress',
            importance     REAL DEFAULT 0.5,
            source_session TEXT,
            consolidated   INTEGER DEFAULT 0,
            decay_at       TEXT,
            is_global      INTEGER DEFAULT 0,
            created_at     TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at     TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS consolidations (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            memory_ids  TEXT NOT NULL,
            insight     TEXT NOT NULL,
            topics      TEXT,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS session_context (
            session_id  TEXT NOT NULL,
            memory_ids  TEXT NOT NULL,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type);
        CREATE INDEX IF NOT EXISTS idx_memories_importance ON memories(importance DESC);
        CREATE INDEX IF NOT EXISTS idx_memories_consolidated ON memories(consolidated);
        CREATE INDEX IF NOT EXISTS idx_memories_decay ON memories(decay_at);
        ",
    )?;

    // FTS5 table — separate call since CREATE VIRTUAL TABLE IF NOT EXISTS
    // may not be supported on all SQLite versions
    let fts_exists: bool = conn.query_row(
        "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='memory_fts'",
        [],
        |row| row.get(0),
    )?;

    if !fts_exists {
        conn.execute_batch(
            "
            CREATE VIRTUAL TABLE memory_fts USING fts5(
                summary, content, entities, topics, semantic_tags,
                content='memories',
                content_rowid='id'
            );

            -- Triggers to keep FTS in sync
            CREATE TRIGGER memory_fts_insert AFTER INSERT ON memories BEGIN
                INSERT INTO memory_fts(rowid, summary, content, entities, topics, semantic_tags)
                VALUES (new.id, new.summary, new.content, new.entities, new.topics, new.semantic_tags);
            END;

            CREATE TRIGGER memory_fts_delete AFTER DELETE ON memories BEGIN
                INSERT INTO memory_fts(memory_fts, rowid, summary, content, entities, topics, semantic_tags)
                VALUES ('delete', old.id, old.summary, old.content, old.entities, old.topics, old.semantic_tags);
            END;

            CREATE TRIGGER memory_fts_update AFTER UPDATE ON memories BEGIN
                INSERT INTO memory_fts(memory_fts, rowid, summary, content, entities, topics, semantic_tags)
                VALUES ('delete', old.id, old.summary, old.content, old.entities, old.topics, old.semantic_tags);
                INSERT INTO memory_fts(rowid, summary, content, entities, topics, semantic_tags)
                VALUES (new.id, new.summary, new.content, new.entities, new.topics, new.semantic_tags);
            END;
            ",
        )?;
    }

    // Schema migrations for existing databases
    migrate(conn)?;

    Ok(())
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    // Add semantic_tags column if missing
    let has_semantic_tags: bool = conn
        .prepare("SELECT semantic_tags FROM memories LIMIT 0")
        .is_ok();
    if !has_semantic_tags {
        conn.execute_batch("ALTER TABLE memories ADD COLUMN semantic_tags TEXT")?;
    }

    // Add is_global column if missing
    let has_is_global: bool = conn
        .prepare("SELECT is_global FROM memories LIMIT 0")
        .is_ok();
    if !has_is_global {
        conn.execute_batch("ALTER TABLE memories ADD COLUMN is_global INTEGER DEFAULT 0")?;
    }

    Ok(())
}
