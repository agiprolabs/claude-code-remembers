use rusqlite::Connection;
use tracing::info;

use crate::db::memories;

/// Delete memories that have passed their decay_at timestamp.
pub fn cleanup_expired(conn: &Connection) -> rusqlite::Result<usize> {
    let deleted = memories::delete_expired(conn)?;
    if deleted > 0 {
        info!("Decay cleanup: removed {deleted} expired memories");
    }
    Ok(deleted)
}
