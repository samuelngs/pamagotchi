use super::*;

const FAILED_INJECTION_ACTION_IDS: &str = "failed_injection_action_ids";

pub(super) fn message_skips_injection_target(msg: &InboundMessage, target_id: &ActionId) -> bool {
    msg.metadata
        .get(FAILED_INJECTION_ACTION_IDS)
        .and_then(serde_json::Value::as_array)
        .is_some_and(|ids| {
            ids.iter()
                .any(|id| id.as_str() == Some(target_id.0.as_str()))
        })
}

pub(super) fn mark_failed_injection_target(
    mut msg: InboundMessage,
    target_id: &ActionId,
) -> InboundMessage {
    let mut obj = match msg.metadata {
        serde_json::Value::Object(obj) => obj,
        serde_json::Value::Null => serde_json::Map::new(),
        other => {
            let mut obj = serde_json::Map::new();
            obj.insert("source_metadata".into(), other);
            obj
        }
    };

    let mut ids = obj
        .remove(FAILED_INJECTION_ACTION_IDS)
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    if !ids
        .iter()
        .any(|id| id.as_str() == Some(target_id.0.as_str()))
    {
        ids.push(serde_json::json!(target_id.0));
    }
    obj.insert(
        FAILED_INJECTION_ACTION_IDS.into(),
        serde_json::Value::Array(ids),
    );
    msg.metadata = serde_json::Value::Object(obj);
    msg
}
