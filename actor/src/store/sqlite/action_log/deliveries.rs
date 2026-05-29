use super::*;

pub(in crate::store::sqlite) fn append_outbound_delivery(
    conn: &Connection,
    delivery: &OutboundDeliveryRecord,
) -> anyhow::Result<()> {
    let conversation_id = delivery.conversation.as_ref().map(|c| c.0.as_str());
    let message_id = delivery.message.as_ref().map(|id| id.0.as_str());
    let channel_id = delivery.channel.as_ref().map(|id| id.0.as_str());
    conn.execute(
        "INSERT INTO action_outbound_deliveries (
            action_id, conversation_id, message_id, channel_id, gateway_id, external_id,
            status, error, attempted_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            delivery.action_id.as_str(),
            conversation_id,
            message_id,
            channel_id,
            delivery.gateway_id.as_str(),
            delivery.external_id.as_str(),
            delivery.status.as_str(),
            delivery.error.as_deref(),
            delivery.attempted_at,
        ],
    )?;
    if let (Some(channel), Some(message)) = (&delivery.channel, &delivery.message) {
        conn.execute(
            "INSERT INTO outbound_deliveries (
                id, action_id, message_id, channel_id, gateway_id, external_id_snapshot,
                status, error, attempted_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                format!("outbound-delivery-{}", nanoid::nanoid!()),
                delivery.action_id.as_str(),
                message.0.as_str(),
                channel.0.as_str(),
                delivery.gateway_id.as_str(),
                delivery.external_id.as_str(),
                delivery.status.as_str(),
                delivery.error.as_deref(),
                delivery.attempted_at,
            ],
        )?;
    }
    Ok(())
}

pub(in crate::store::sqlite) fn outbound_deliveries_for_action(
    conn: &Connection,
    action_id: &str,
) -> anyhow::Result<Vec<OutboundDeliveryRecord>> {
    let mut stmt = conn.prepare(
        "SELECT action_id, conversation_id, message_id, channel_id, gateway_id, external_id,
                status, error, attempted_at
         FROM action_outbound_deliveries
         WHERE action_id = ?1
         ORDER BY attempted_at ASC",
    )?;
    let results = stmt
        .query_map(params![action_id], |row| {
            let conversation: Option<String> = row.get("conversation_id")?;
            let message: Option<String> = row.get("message_id")?;
            let channel: Option<String> = row.get("channel_id")?;
            Ok(OutboundDeliveryRecord {
                action_id: row.get("action_id")?,
                conversation: conversation.map(ConversationId),
                message: message.map(MessageId),
                channel: channel.map(ChannelId),
                gateway_id: row.get("gateway_id")?,
                external_id: row.get("external_id")?,
                status: row.get("status")?,
                error: row.get("error")?,
                attempted_at: row.get("attempted_at")?,
            })
        })?
        .filter_map(|row| row.ok())
        .collect();
    Ok(results)
}
