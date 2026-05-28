use super::*;

pub(in crate::store::sqlite) fn append_outbound_delivery(
    conn: &Connection,
    delivery: &OutboundDeliveryRecord,
) -> anyhow::Result<()> {
    let conversation_id = delivery.conversation.as_ref().map(|c| c.0.as_str());
    conn.execute(
        "INSERT INTO action_outbound_deliveries (
            action_id, conversation_id, gateway_id, external_id, status, error, attempted_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            delivery.action_id.as_str(),
            conversation_id,
            delivery.gateway_id.as_str(),
            delivery.external_id.as_str(),
            delivery.status.as_str(),
            delivery.error.as_deref(),
            delivery.attempted_at,
        ],
    )?;
    Ok(())
}

pub(in crate::store::sqlite) fn outbound_deliveries_for_action(
    conn: &Connection,
    action_id: &str,
) -> anyhow::Result<Vec<OutboundDeliveryRecord>> {
    let mut stmt = conn.prepare(
        "SELECT action_id, conversation_id, gateway_id, external_id, status, error, attempted_at
         FROM action_outbound_deliveries
         WHERE action_id = ?1
         ORDER BY attempted_at ASC",
    )?;
    let results = stmt
        .query_map(params![action_id], |row| {
            let conversation: Option<String> = row.get("conversation_id")?;
            Ok(OutboundDeliveryRecord {
                action_id: row.get("action_id")?,
                conversation: conversation.map(ConversationId),
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
