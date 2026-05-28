use super::*;

impl Mind {
    pub(super) async fn apply_message_edit(
        &self,
        conversation: &protocol::ConversationId,
        gateway_id: &str,
        message_id: &str,
        content: &str,
        edited_at: i64,
    ) {
        match self
            .store
            .update_message_content_by_source(
                conversation,
                gateway_id,
                message_id,
                content,
                edited_at,
            )
            .await
        {
            Ok(true) => info!(
                conversation = %conversation.0,
                gateway = %gateway_id,
                message_id,
                "applied message edit"
            ),
            Ok(false) => warn!(
                conversation = %conversation.0,
                gateway = %gateway_id,
                message_id,
                "message edit did not match stored message"
            ),
            Err(e) => warn!(
                %e,
                conversation = %conversation.0,
                gateway = %gateway_id,
                message_id,
                "failed to apply message edit"
            ),
        }
    }

    pub(super) async fn apply_message_delete(
        &self,
        conversation: &protocol::ConversationId,
        gateway_id: &str,
        message_id: &str,
        deleted_at: i64,
    ) {
        match self
            .store
            .mark_message_deleted_by_source(conversation, gateway_id, message_id, deleted_at)
            .await
        {
            Ok(true) => info!(
                conversation = %conversation.0,
                gateway = %gateway_id,
                message_id,
                "applied message delete"
            ),
            Ok(false) => warn!(
                conversation = %conversation.0,
                gateway = %gateway_id,
                message_id,
                "message delete did not match stored message"
            ),
            Err(e) => warn!(
                %e,
                conversation = %conversation.0,
                gateway = %gateway_id,
                message_id,
                "failed to apply message delete"
            ),
        }
    }
}
