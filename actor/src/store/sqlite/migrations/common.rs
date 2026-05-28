use rusqlite::Connection;
use std::collections::HashSet;

pub(super) fn table_columns(conn: &Connection, table: &str) -> anyhow::Result<HashSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>("name"))?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(columns)
}
