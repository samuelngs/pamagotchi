use super::common;
use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    let columns = common::table_columns(conn, "messages")?;

    for (name, definition) in [
        ("source_gateway_id", "TEXT"),
        ("source_message_id", "TEXT"),
        ("sender_external_id", "TEXT"),
        ("reply_external_id", "TEXT"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE messages ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_source_unique
            ON messages(conversation_id, source_gateway_id, source_message_id, role)
            WHERE source_gateway_id IS NOT NULL AND source_message_id IS NOT NULL;",
    )?;

    Ok(())
}
