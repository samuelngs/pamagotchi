use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "DELETE FROM action_messages
            WHERE id IN (
                SELECT later.id
                FROM action_messages later
                JOIN action_messages earlier
                    ON earlier.action_id = later.action_id
                    AND earlier.role = later.role
                    AND earlier.source_gateway_id = later.source_gateway_id
                    AND earlier.source_message_id = later.source_message_id
                    AND earlier.id < later.id
                WHERE later.source_gateway_id IS NOT NULL
                    AND later.source_message_id IS NOT NULL
            );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_action_messages_source_unique
            ON action_messages(action_id, role, source_gateway_id, source_message_id)
            WHERE source_gateway_id IS NOT NULL AND source_message_id IS NOT NULL;",
    )?;
    Ok(())
}
