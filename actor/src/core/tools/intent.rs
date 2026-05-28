use super::context::SessionContext;
use crate::state::Authority;
use crate::store::{IntentRecord, IntentUpdateRecord};
use inference::Tool;
use protocol::{ConversationId, MemoryId, PersonId, ProfileId};
use serde_json::{Value, json};

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
                    "profile": {
                        "type": "string",
                        "description": "Profile ID this intent relates to"
                    },
                    "conversation": {
                        "type": "string",
                        "description": "Conversation ID for context"
                    },
                    "recurrence": {
                        "type": "string",
                        "description": "Optional recurrence rule for future expansion"
                    },
                    "priority": {
                        "type": "integer",
                        "description": "0 to 100 priority. Higher fires first when multiple intents are due.",
                        "default": 50
                    },
                    "dedupe_key": {
                        "type": "string",
                        "description": "Optional stable key to avoid duplicate equivalent intents"
                    },
                    "source_memory_id": {
                        "type": "string",
                        "description": "Optional memory id that explains why this intent exists, such as a commitment or open-loop memory."
                    },
                    "sensitive": {
                        "type": "boolean",
                        "description": "Set true when the follow-up involves private, medical, legal, financial, identity, credential, or otherwise sensitive content. Sensitive outreach requires chosen-person approval."
                    },
                    "requires_chosen_person_approval": {
                        "type": "boolean",
                        "description": "Set true when this intent should not proactively contact anyone until the chosen person has approved it."
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
                    "profile": {
                        "type": "string",
                        "description": "New profile ID"
                    },
                    "conversation": {
                        "type": "string",
                        "description": "New conversation ID"
                    },
                    "recurrence": {
                        "type": "string",
                        "description": "New recurrence rule"
                    },
                    "status": {
                        "type": "string",
                        "enum": ["active", "pending_approval", "fired", "completed", "cancelled"],
                        "description": "New intent status"
                    },
                    "priority": {
                        "type": "integer",
                        "description": "New priority from 0 to 100"
                    },
                    "dedupe_key": {
                        "type": "string",
                        "description": "New dedupe key"
                    },
                    "source_memory_id": {
                        "type": "string",
                        "description": "Memory id that explains why this intent exists."
                    },
                    "sensitive": {
                        "type": "boolean",
                        "description": "Set true when the updated follow-up involves sensitive content. Sensitive outreach requires chosen-person approval."
                    },
                    "requires_chosen_person_approval": {
                        "type": "boolean",
                        "description": "Set true when this intent should not proactively contact anyone until the chosen person has approved it."
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

pub async fn create(args: &Value, ctx: &SessionContext) -> String {
    let Some(task) = args["task"].as_str().filter(|s| !s.trim().is_empty()) else {
        return json!({"status": "error", "message": "Provide task."}).to_string();
    };
    let kind = args["kind"].as_str().unwrap_or("scheduled");
    if !matches!(kind, "scheduled" | "triggered") {
        return json!({"status": "error", "message": "kind must be scheduled or triggered."})
            .to_string();
    }

    let fire_at = args["fire_at"].as_i64();
    let condition = args["condition"].as_str().map(str::to_string);
    if kind == "scheduled" && fire_at.is_none() {
        return json!({"status": "error", "message": "scheduled intents require fire_at."})
            .to_string();
    }
    if kind == "triggered" && condition.as_deref().is_none_or(str::is_empty) {
        return json!({"status": "error", "message": "triggered intents require condition."})
            .to_string();
    }

    let now = super::util::now();
    let chosen_person_approved = matches!(ctx.authority, Authority::ChosenPerson);
    let status = if super::permission::intent_requires_chosen_person_approval(args)
        && !chosen_person_approved
    {
        "pending_approval"
    } else {
        "active"
    };
    let intent = IntentRecord {
        id: format!("intent-{}", super::util::uuid_v4()),
        kind: kind.into(),
        status: status.into(),
        task: task.into(),
        person: args["person"]
            .as_str()
            .map(|id| PersonId(id.to_string()))
            .or_else(|| ctx.messages.first().and_then(|msg| msg.person.clone())),
        profile: args["profile"]
            .as_str()
            .map(|id| ProfileId(id.to_string()))
            .or_else(|| ctx.messages.first().and_then(|msg| msg.profile.clone())),
        conversation: args["conversation"]
            .as_str()
            .map(|id| ConversationId(id.to_string()))
            .or_else(|| ctx.conversation.clone()),
        fire_at,
        condition,
        recurrence: args["recurrence"].as_str().map(str::to_string),
        priority: args["priority"].as_u64().unwrap_or(50).min(100) as u8,
        dedupe_key: args["dedupe_key"].as_str().map(str::to_string),
        source_action: Some(ctx.action_id.0.clone()),
        source_memory: source_memory_arg(args),
        created_at: now,
        updated_at: now,
        last_fired_at: None,
        chosen_person_approved,
    };

    if intent.status == "pending_approval" {
        return create_pending_chosen_person_approval_intent(intent, args, ctx, now).await;
    }

    match ctx.store.create_intent(&intent).await {
        Ok(()) => json!({
            "status": "created",
            "intent_id": intent.id,
        })
        .to_string(),
        Err(e) => json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string(),
    }
}

async fn create_pending_chosen_person_approval_intent(
    pending_intent: IntentRecord,
    args: &Value,
    ctx: &SessionContext,
    now: i64,
) -> String {
    let Some(chosen_person) = chosen_person(ctx) else {
        return json!({
            "status": "error",
            "message": "Chosen-person approval is required, but no chosen person is configured."
        })
        .to_string();
    };

    let pending_id = pending_intent.id.clone();
    let pending_task = pending_intent.task.clone();
    let original_dedupe_key = pending_intent
        .dedupe_key
        .clone()
        .unwrap_or_else(|| format!("intent-tool:pending-approval:{pending_id}"));
    if let Err(e) = ctx.store.create_intent(&pending_intent).await {
        return json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string();
    }

    let approval_intent = IntentRecord {
        id: format!("intent-{}", super::util::uuid_v4()),
        kind: "scheduled".into(),
        status: "active".into(),
        task: format!(
            "Review proactive outreach before it is sent. Pending intent: {pending_id}. Proposed task: {pending_task}. {} If the chosen person approves, update intent {pending_id} with status active. If the chosen person declines, delete intent {pending_id}.",
            chosen_person_approval_target_description(&pending_intent, args),
        ),
        person: Some(chosen_person),
        profile: None,
        conversation: None,
        fire_at: Some(now),
        condition: None,
        recurrence: None,
        priority: 100,
        dedupe_key: Some(format!(
            "chosen-person-approval:intent-tool:{original_dedupe_key}"
        )),
        source_action: Some(ctx.action_id.0.clone()),
        source_memory: pending_intent.source_memory.clone(),
        created_at: now,
        updated_at: now,
        last_fired_at: None,
        chosen_person_approved: true,
    };
    let chosen_person_intent_id = approval_intent.id.clone();
    if let Err(e) = ctx.store.create_intent(&approval_intent).await {
        return json!({
            "status": "error",
            "message": format!("Created pending intent {pending_id}, but failed to create chosen-person approval intent: {e}"),
            "intent_id": pending_id,
        })
        .to_string();
    }

    json!({
        "status": "pending_approval",
        "intent_id": pending_id,
        "chosen_person_intent_id": chosen_person_intent_id,
    })
    .to_string()
}

fn chosen_person(ctx: &SessionContext) -> Option<PersonId> {
    ctx.state
        .read_state()
        .bonds
        .iter()
        .find(|(_, relationship)| matches!(relationship.authority, Authority::ChosenPerson))
        .map(|(person, _)| person.clone())
}

fn chosen_person_approval_target_description(intent: &IntentRecord, args: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(person) = &intent.person {
        parts.push(format!("Target person: {}.", person.0));
    }
    if let Some(profile) = &intent.profile {
        parts.push(format!("Target profile: {}.", profile.0));
    }
    if let Some(conversation) = &intent.conversation {
        parts.push(format!("Conversation: {}.", conversation.0));
    }
    if args["sensitive"].as_bool().unwrap_or(false) {
        parts.push("The request was marked sensitive.".into());
    }
    if args["requires_chosen_person_approval"]
        .as_bool()
        .unwrap_or(false)
    {
        parts.push("The request explicitly requires chosen-person approval.".into());
    }
    if parts.is_empty() {
        "No explicit target was provided.".into()
    } else {
        parts.join(" ")
    }
}

pub async fn update(args: &Value, ctx: &SessionContext) -> String {
    let id = args["intent_id"].as_str().unwrap_or("");
    if id.is_empty() {
        return json!({"status": "error", "message": "Provide intent_id."}).to_string();
    }
    let kind = args["kind"].as_str();
    if kind.is_some_and(|kind| !matches!(kind, "scheduled" | "triggered")) {
        return json!({"status": "error", "message": "kind must be scheduled or triggered."})
            .to_string();
    }

    let is_chosen_person = matches!(ctx.authority, Authority::ChosenPerson);
    if !is_chosen_person && args["status"].as_str() == Some("active") {
        match ctx.store.get_intent(id).await {
            Ok(Some(intent)) if intent.status == "pending_approval" => {
                return json!({
                    "status": "error",
                    "message": "Activating an chosen-person-approval intent requires chosen-person authority.",
                })
                .to_string();
            }
            Ok(_) => {}
            Err(e) => {
                return json!({
                    "status": "error",
                    "message": format!("Could not verify intent chosen-person approval status: {e}"),
                })
                .to_string();
            }
        }
    }

    let chosen_person_approved = if is_chosen_person {
        Some(true)
    } else if update_changes_approved_intent_surface(args) {
        Some(false)
    } else {
        None
    };
    let update = IntentUpdateRecord {
        kind: kind.map(str::to_string),
        status: args["status"].as_str().map(str::to_string),
        task: args["task"].as_str().map(str::to_string),
        person: args["person"].as_str().map(|id| PersonId(id.to_string())),
        profile: args["profile"].as_str().map(|id| ProfileId(id.to_string())),
        conversation: args["conversation"]
            .as_str()
            .map(|id| ConversationId(id.to_string())),
        fire_at: args["fire_at"].as_i64(),
        condition: args["condition"].as_str().map(str::to_string),
        recurrence: args["recurrence"].as_str().map(str::to_string),
        priority: args["priority"].as_u64().map(|v| v.min(100) as u8),
        dedupe_key: args["dedupe_key"].as_str().map(str::to_string),
        source_memory: source_memory_arg(args),
        chosen_person_approved,
        updated_at: super::util::now(),
    };

    match ctx.store.update_intent(id, &update).await {
        Ok(true) => json!({"status": "updated", "intent_id": id}).to_string(),
        Ok(false) => json!({"status": "not_found", "intent_id": id}).to_string(),
        Err(e) => json!({"status": "error", "message": format!("{e}")}).to_string(),
    }
}

fn update_changes_approved_intent_surface(args: &Value) -> bool {
    args.as_object().is_some_and(|object| {
        object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "intent_id" | "sensitive" | "requires_chosen_person_approval"
            )
        })
    })
}

fn source_memory_arg(args: &Value) -> Option<MemoryId> {
    args["source_memory_id"]
        .as_str()
        .or_else(|| args["source_memory"].as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| MemoryId(id.to_string()))
}

pub async fn delete(args: &Value, ctx: &SessionContext) -> String {
    let id = args["intent_id"].as_str().unwrap_or("");
    if id.is_empty() {
        return json!({"status": "error", "message": "Provide intent_id."}).to_string();
    }
    match ctx.store.cancel_intent(id, super::util::now()).await {
        Ok(true) => json!({"status": "cancelled", "intent_id": id}).to_string(),
        Ok(false) => json!({"status": "not_found", "intent_id": id}).to_string(),
        Err(e) => json!({"status": "error", "message": format!("{e}")}).to_string(),
    }
}

#[cfg(test)]
mod tests;
