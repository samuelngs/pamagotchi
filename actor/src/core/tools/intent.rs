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
                        "description": "Set true when the follow-up involves private, medical, legal, financial, identity, credential, or otherwise sensitive content. Sensitive outreach requires owner approval."
                    },
                    "requires_owner_approval": {
                        "type": "boolean",
                        "description": "Set true when this intent should not proactively contact anyone until the owner has approved it."
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
                        "description": "Set true when the updated follow-up involves sensitive content. Sensitive outreach requires owner approval."
                    },
                    "requires_owner_approval": {
                        "type": "boolean",
                        "description": "Set true when this intent should not proactively contact anyone until the owner has approved it."
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
    let owner_approved = matches!(ctx.authority, Authority::Owner);
    let status = if super::permission::intent_requires_owner_approval(args) && !owner_approved {
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
        owner_approved,
    };

    if intent.status == "pending_approval" {
        return create_pending_owner_approval_intent(intent, args, ctx, now).await;
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

async fn create_pending_owner_approval_intent(
    pending_intent: IntentRecord,
    args: &Value,
    ctx: &SessionContext,
    now: i64,
) -> String {
    let Some(owner) = owner_person(ctx) else {
        return json!({
            "status": "error",
            "message": "Owner approval is required, but no owner person is configured."
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
            "Review proactive outreach before it is sent. Pending intent: {pending_id}. Proposed task: {pending_task}. {} If the owner approves, update intent {pending_id} with status active. If the owner declines, delete intent {pending_id}.",
            owner_approval_target_description(&pending_intent, args),
        ),
        person: Some(owner),
        profile: None,
        conversation: None,
        fire_at: Some(now),
        condition: None,
        recurrence: None,
        priority: 100,
        dedupe_key: Some(format!("owner-approval:intent-tool:{original_dedupe_key}")),
        source_action: Some(ctx.action_id.0.clone()),
        source_memory: pending_intent.source_memory.clone(),
        created_at: now,
        updated_at: now,
        last_fired_at: None,
        owner_approved: true,
    };
    let owner_intent_id = approval_intent.id.clone();
    if let Err(e) = ctx.store.create_intent(&approval_intent).await {
        return json!({
            "status": "error",
            "message": format!("Created pending intent {pending_id}, but failed to create owner approval intent: {e}"),
            "intent_id": pending_id,
        })
        .to_string();
    }

    json!({
        "status": "pending_approval",
        "intent_id": pending_id,
        "owner_intent_id": owner_intent_id,
    })
    .to_string()
}

fn owner_person(ctx: &SessionContext) -> Option<PersonId> {
    ctx.state
        .read_state()
        .bonds
        .iter()
        .find(|(_, relationship)| matches!(relationship.authority, Authority::Owner))
        .map(|(person, _)| person.clone())
}

fn owner_approval_target_description(intent: &IntentRecord, args: &Value) -> String {
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
    if args["requires_owner_approval"].as_bool().unwrap_or(false) {
        parts.push("The request explicitly requires owner approval.".into());
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

    let is_owner = matches!(ctx.authority, Authority::Owner);
    if !is_owner && args["status"].as_str() == Some("active") {
        match ctx.store.get_intent(id).await {
            Ok(Some(intent)) if intent.status == "pending_approval" => {
                return json!({
                    "status": "error",
                    "message": "Activating an owner-approval intent requires owner authority.",
                })
                .to_string();
            }
            Ok(_) => {}
            Err(e) => {
                return json!({
                    "status": "error",
                    "message": format!("Could not verify intent owner approval status: {e}"),
                })
                .to_string();
            }
        }
    }

    let owner_approved = if is_owner {
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
        owner_approved,
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
                "intent_id" | "sensitive" | "requires_owner_approval"
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
mod tests {
    use super::{create, tools, update};
    use crate::core::action::{ActionId, ActionKind, RunningState};
    use crate::core::handle::{SharedState, StateHandle};
    use crate::core::tools::context::{SessionContext, SessionKind};
    use crate::state::{ActorState, Authority, GrowthConfig};
    use crate::store::{SqliteStore, Store};
    use gateway::GatewayRouter;
    use inference::{
        AssistantMessage, Capability, ChatRequest, ChatResponse, ChatStream, FinishReason,
        InferenceEndpoint, InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge,
        Reasoning, SamplingConfig, Usage,
    };
    use protocol::{ConversationId, InboundMessage, PersonId};
    use std::sync::{Arc, RwLock};
    use tokio::sync::mpsc;

    struct NoopBridge;

    #[async_trait::async_trait]
    impl OpenAiCompatibleBridge for NoopBridge {
        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
            Ok(ChatResponse {
                message: AssistantMessage {
                    text: Some(String::new()),
                    reasoning_content: None,
                    tool_calls: vec![],
                },
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    input_tokens: 0,
                    output_tokens: 0,
                },
            })
        }

        async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
            anyhow::bail!("noop bridge is not used by intent tests")
        }
    }

    fn test_context(
        store: Arc<SqliteStore>,
        authority: Authority,
        current_person: Option<PersonId>,
        owner: Option<PersonId>,
    ) -> SessionContext {
        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let mut actor = ActorState::new(Default::default());
        if let Some(owner) = owner {
            actor.set_relationship_config(&owner, Some(Authority::Owner));
        }
        let shared = Arc::new(SharedState {
            actor: RwLock::new(actor),
            config: RwLock::new(GrowthConfig::default()),
        });
        let router = InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                model: "noop".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap();

        SessionContext {
            action_id: ActionId("intent-test-action".into()),
            kind: SessionKind::Action(ActionKind::Respond),
            messages: vec![InboundMessage {
                message_id: "msg-1".into(),
                gateway_id: "relay".into(),
                sender_external_id: "local".into(),
                sender_display_name: None,
                reply_external_id: "local".into(),
                conversation: ConversationId("relay:local".into()),
                group: None,
                identity: None,
                profile: None,
                person: current_person,
                content: "hello".into(),
                attachments: vec![],
                timestamp: 1000,
                metadata: serde_json::Value::Null,
            }],
            conversation: Some(ConversationId("relay:local".into())),
            authority,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: StateHandle::new(shared, delta_tx),
            store,
            media_store: None,
            router: Arc::new(router),
            endpoints: vec![],
            reasoning: Reasoning::Basic,
            inject_rx,
            progress: Arc::new(RwLock::new(RunningState::new())),
            max_turns: 1,
            max_action_attempts: 1,
            escalate_after: 1,
            gateway: Arc::new(GatewayRouter::new()),
            typing: Arc::new(RwLock::new(Default::default())),
            metrics: Arc::new(crate::core::ActorMetrics::default()),
            session_start: std::time::Instant::now(),
        }
    }

    #[test]
    fn intent_tools_expose_source_memory_attribution() {
        for tool_name in ["create_intent", "update_intent"] {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == tool_name)
                .expect("intent tool exists");
            assert!(
                tool.parameters["properties"]
                    .get("source_memory_id")
                    .is_some()
            );
        }
    }

    #[tokio::test]
    async fn create_sensitive_current_intent_routes_to_owner_approval() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let owner = PersonId("person-owner".into());
        let current = PersonId("person-current".into());
        let ctx = test_context(
            store.clone(),
            Authority::Default,
            Some(current.clone()),
            Some(owner.clone()),
        );
        let args = serde_json::json!({
            "task": "Ask Sam about the private medical update",
            "kind": "scheduled",
            "fire_at": 1200,
            "person": current.0.clone(),
            "dedupe_key": "intent:test:private-medical-update"
        });

        crate::core::tools::permission::check("create_intent", &args, &ctx)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&create(&args, &ctx).await).unwrap();

        assert_eq!(value["status"], "pending_approval");
        let pending_id = value["intent_id"].as_str().unwrap();
        let owner_intent_id = value["owner_intent_id"].as_str().unwrap();
        let pending = store.get_intent(pending_id).await.unwrap().unwrap();
        assert_eq!(pending.status, "pending_approval");
        assert!(!pending.owner_approved);
        assert_eq!(pending.person.as_ref(), Some(&current));

        let owner_intent = store.get_intent(owner_intent_id).await.unwrap().unwrap();
        assert_eq!(owner_intent.status, "active");
        assert!(owner_intent.owner_approved);
        assert_eq!(owner_intent.person.as_ref(), Some(&owner));
        assert!(owner_intent.task.contains(pending_id));
        assert!(owner_intent.task.contains("private medical update"));
    }

    #[tokio::test]
    async fn non_owner_cannot_activate_pending_owner_approval_intent() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let owner = PersonId("person-owner".into());
        let current = PersonId("person-current".into());
        let ctx = test_context(
            store.clone(),
            Authority::Default,
            Some(current.clone()),
            Some(owner.clone()),
        );
        let create_args = serde_json::json!({
            "task": "Ask Sam about the private medical update",
            "kind": "scheduled",
            "fire_at": 1200,
            "person": current.0.clone()
        });
        let created: serde_json::Value =
            serde_json::from_str(&create(&create_args, &ctx).await).unwrap();
        let pending_id = created["intent_id"].as_str().unwrap();

        let denied: serde_json::Value = serde_json::from_str(
            &update(
                &serde_json::json!({
                    "intent_id": pending_id,
                    "status": "active"
                }),
                &ctx,
            )
            .await,
        )
        .unwrap();

        assert_eq!(denied["status"], "error");
        assert!(
            denied["message"]
                .as_str()
                .unwrap()
                .contains("requires owner authority")
        );
        let pending = store.get_intent(pending_id).await.unwrap().unwrap();
        assert_eq!(pending.status, "pending_approval");
        assert!(!pending.owner_approved);

        let owner_ctx = test_context(
            store.clone(),
            Authority::Owner,
            Some(current.clone()),
            Some(owner),
        );
        let approved: serde_json::Value = serde_json::from_str(
            &update(
                &serde_json::json!({
                    "intent_id": pending_id,
                    "status": "active"
                }),
                &owner_ctx,
            )
            .await,
        )
        .unwrap();
        assert_eq!(approved["status"], "updated");
        let intent = store.get_intent(pending_id).await.unwrap().unwrap();
        assert_eq!(intent.status, "active");
        assert!(intent.owner_approved);
    }
}
