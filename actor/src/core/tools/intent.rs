use inference::Tool;
use serde_json::{json, Value};
use tracing::info;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "create_intent".into(),
            description: "Schedule something for later. A reminder, follow-up, or triggered action.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "What to do when the intent fires"
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["scheduled", "triggered"],
                        "description": "scheduled = at a specific time, triggered = when a condition is met"
                    },
                    "fire_at": {
                        "type": "integer",
                        "description": "Unix timestamp for scheduled intents"
                    },
                    "condition": {
                        "type": "string",
                        "description": "Natural language condition for triggered intents, e.g. 'next time Sam messages'"
                    },
                    "person": {
                        "type": "string",
                        "description": "Person ID this intent relates to"
                    },
                    "conversation": {
                        "type": "string",
                        "description": "Conversation ID for context"
                    }
                },
                "required": ["task", "kind"]
            }),
        },
        Tool {
            name: "update_intent".into(),
            description: "Modify an existing intent. Atomic update — safer than delete + create if the program crashes between operations.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "intent_id": {
                        "type": "string",
                        "description": "ID of the intent to update"
                    },
                    "task": {
                        "type": "string",
                        "description": "New task description"
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["scheduled", "triggered"],
                        "description": "New intent kind"
                    },
                    "fire_at": {
                        "type": "integer",
                        "description": "New fire time (unix timestamp) for scheduled intents"
                    },
                    "condition": {
                        "type": "string",
                        "description": "New condition for triggered intents"
                    },
                    "person": {
                        "type": "string",
                        "description": "New person ID"
                    },
                    "conversation": {
                        "type": "string",
                        "description": "New conversation ID"
                    }
                },
                "required": ["intent_id"]
            }),
        },
        Tool {
            name: "delete_intent".into(),
            description: "Cancel a scheduled or triggered intent.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "intent_id": {
                        "type": "string",
                        "description": "ID of the intent to cancel"
                    }
                },
                "required": ["intent_id"]
            }),
        },
    ]
}

pub async fn create(args: &Value) -> String {
    let task = args["task"].as_str().unwrap_or("");
    let kind = args["kind"].as_str().unwrap_or("scheduled");

    // TODO: persist intent to store (intent table not yet built)
    info!(task, kind, "intent created (stub)");

    format!("Intent created: {task}")
}

pub async fn update(args: &Value) -> String {
    let id = args["intent_id"].as_str().unwrap_or("");
    let task = args["task"].as_str();
    let kind = args["kind"].as_str();

    // TODO: persist to store (intent table not yet built)
    info!(
        intent_id = id,
        new_task = ?task,
        new_kind = ?kind,
        "intent updated (stub)"
    );

    format!("Intent {id} updated.")
}

pub async fn delete(args: &Value) -> String {
    let id = args["intent_id"].as_str().unwrap_or("");

    // TODO: persist to store (intent table not yet built)
    info!(intent_id = id, "intent deleted (stub)");

    format!("Intent {id} deleted.")
}
