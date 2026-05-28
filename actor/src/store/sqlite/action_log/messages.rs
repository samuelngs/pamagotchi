use super::*;

pub(in crate::store::sqlite) fn append_action_message(
    conn: &Connection,
    message: &ActionMessageRecord,
) -> anyhow::Result<()> {
    let conversation_id = message.conversation.as_ref().map(|c| c.0.as_str());
    let source_gateway_id = message.source_gateway_id.as_deref();
    let source_message_id = message.source_message_id.as_deref();
    let sender_external_id = message.sender_external_id.as_deref();
    let reply_external_id = message.reply_external_id.as_deref();
    let content = message.content.as_deref();
    conn.execute(
        "INSERT OR IGNORE INTO action_messages (
            action_id, role, conversation_id, source_gateway_id, source_message_id,
            sender_external_id, reply_external_id, content, created_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            message.action_id.as_str(),
            message.role.as_str(),
            conversation_id,
            source_gateway_id,
            source_message_id,
            sender_external_id,
            reply_external_id,
            content,
            message.created_at,
        ],
    )?;
    Ok(())
}
