use super::common;
use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    let columns = common::table_columns(conn, "intents")?;
    if !columns.contains("chosen_person_approved") {
        conn.execute(
            "ALTER TABLE intents ADD COLUMN chosen_person_approved INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}
