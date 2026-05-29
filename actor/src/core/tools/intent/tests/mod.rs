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
    chosen_human: Option<PersonId>,
) -> SessionContext {
    let (_inject_tx, inject_rx) = mpsc::channel(1);
    let (delta_tx, _delta_rx) = mpsc::channel(1);
    let mut actor = ActorState::new(Default::default());
    if let Some(chosen_human) = chosen_human {
        actor.set_relationship_config(&chosen_human, Some(Authority::ChosenHuman));
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
async fn create_sensitive_current_intent_routes_to_chosen_human_approval() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let chosen_human = PersonId("person-chosen_human".into());
    let current = PersonId("person-current".into());
    let ctx = test_context(
        store.clone(),
        Authority::Default,
        Some(current.clone()),
        Some(chosen_human.clone()),
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
    let chosen_human_intent_id = value["chosen_human_intent_id"].as_str().unwrap();
    let pending = store.get_intent(pending_id).await.unwrap().unwrap();
    assert_eq!(pending.status, "pending_approval");
    assert!(!pending.chosen_human_approved);
    assert_eq!(pending.person.as_ref(), Some(&current));

    let chosen_human_intent = store
        .get_intent(chosen_human_intent_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(chosen_human_intent.status, "active");
    assert!(chosen_human_intent.chosen_human_approved);
    assert_eq!(chosen_human_intent.person.as_ref(), Some(&chosen_human));
    assert!(chosen_human_intent.task.contains(pending_id));
    assert!(chosen_human_intent.task.contains("private medical update"));
}

#[tokio::test]
async fn non_chosen_human_cannot_activate_pending_chosen_human_approval_intent() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let chosen_human = PersonId("person-chosen_human".into());
    let current = PersonId("person-current".into());
    let ctx = test_context(
        store.clone(),
        Authority::Default,
        Some(current.clone()),
        Some(chosen_human.clone()),
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
            .contains("requires chosen-human authority")
    );
    let pending = store.get_intent(pending_id).await.unwrap().unwrap();
    assert_eq!(pending.status, "pending_approval");
    assert!(!pending.chosen_human_approved);

    let chosen_human_ctx = test_context(
        store.clone(),
        Authority::ChosenHuman,
        Some(current.clone()),
        Some(chosen_human),
    );
    let approved: serde_json::Value = serde_json::from_str(
        &update(
            &serde_json::json!({
                "intent_id": pending_id,
                "status": "active"
            }),
            &chosen_human_ctx,
        )
        .await,
    )
    .unwrap();
    assert_eq!(approved["status"], "updated");
    let intent = store.get_intent(pending_id).await.unwrap().unwrap();
    assert_eq!(intent.status, "active");
    assert!(intent.chosen_human_approved);
}
