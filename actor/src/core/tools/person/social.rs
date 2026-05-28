use super::super::context::{SessionContext, SessionState};
use crate::identity::{
    Relation, RelationDirection, RelationSource, RelationStatus, SocialRelation,
};
use protocol::{InboundMessage, PersonId};
use serde_json::{Value, json};

pub async fn upsert_social_relation(
    args: &Value,
    ctx: &SessionContext,
    state: &SessionState,
) -> String {
    let Some(person_a) = args["person_a"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(|id| PersonId(id.to_string()))
    else {
        return json!({
            "status": "error",
            "message": "person_a is required.",
        })
        .to_string();
    };
    let Some(person_b) = args["person_b"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(|id| PersonId(id.to_string()))
    else {
        return json!({
            "status": "error",
            "message": "person_b is required.",
        })
        .to_string();
    };
    if person_a == person_b {
        return json!({
            "status": "error",
            "message": "Social relations require two distinct people.",
        })
        .to_string();
    }

    let relation = args["relation"]
        .as_str()
        .filter(|relation| !relation.trim().is_empty())
        .map(Relation::parse)
        .unwrap_or_else(|| Relation::Custom("related".into()));
    let direction = args["direction"]
        .as_str()
        .and_then(RelationDirection::parse)
        .unwrap_or_else(|| relation.default_direction());
    let confidence = args["confidence"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0) as f32;
    let status = args["status"]
        .as_str()
        .map(RelationStatus::parse)
        .unwrap_or(RelationStatus::Stated);
    let source_kind = args["source_kind"]
        .as_str()
        .map(RelationSource::parse)
        .unwrap_or(RelationSource::Stated);
    if let Some(missing) = missing_relation_evidence_message_ids(args, ctx, state) {
        return json!({
            "status": "error",
            "message": "Explicit social relation evidence_message_ids must reference messages available to the current action.",
            "missing_evidence_message_ids": missing,
        })
        .to_string();
    }
    let asserted_by = relation_asserted_by_person(args, ctx, state, &source_kind);
    let now = super::super::util::now();

    let relation = SocialRelation {
        person_a: person_a.clone(),
        person_b: person_b.clone(),
        relation,
        direction,
        confidence,
        status,
        evidence: Some(relation_evidence(args, ctx, state)),
        source_kind,
        asserted_by,
        created_at: now,
        updated_at: now,
    };

    match ctx.store.upsert_relation(&relation).await {
        Ok(()) => json!({
            "status": "updated",
            "person_a": person_a.0,
            "person_b": person_b.0,
            "relation": relation.relation.as_str(),
            "confidence": relation.confidence,
            "relation_status": relation.status.as_str(),
            "source_kind": relation.source_kind.as_str(),
        })
        .to_string(),
        Err(e) => json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string(),
    }
}

fn relation_evidence(args: &Value, ctx: &SessionContext, state: &SessionState) -> Value {
    let supplied = args
        .get("evidence")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    let mut evidence = json!({
        "action_id": ctx.action_id.0,
        "message_ids": relation_evidence_message_ids(args, ctx, state),
        "evidence": supplied,
    });
    if let Some(quote) = args["evidence_quote"]
        .as_str()
        .map(str::trim)
        .filter(|quote| !quote.is_empty())
    {
        evidence["quote"] = json!(quote);
    }
    evidence
}

fn relation_evidence_message_ids(
    args: &Value,
    ctx: &SessionContext,
    state: &SessionState,
) -> Vec<String> {
    let supplied = explicit_relation_evidence_message_ids(args);
    if !supplied.is_empty() {
        return supplied;
    }
    evidence_source_messages(ctx, state)
        .iter()
        .map(|message| message.message_id.clone())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>()
}

fn explicit_relation_evidence_message_ids(args: &Value) -> Vec<String> {
    let supplied = string_array(&args["evidence_message_ids"]).collect::<Vec<_>>();
    if !supplied.is_empty() {
        return supplied;
    }
    if let Some(ids) = args
        .get("evidence")
        .and_then(|evidence| evidence.get("message_ids"))
        .map(|value| string_array(value).collect::<Vec<_>>())
        .filter(|ids| !ids.is_empty())
    {
        return ids;
    }
    vec![]
}

fn missing_relation_evidence_message_ids(
    args: &Value,
    ctx: &SessionContext,
    state: &SessionState,
) -> Option<Vec<String>> {
    let supplied = explicit_relation_evidence_message_ids(args);
    if supplied.is_empty() {
        return None;
    }
    let messages = evidence_source_messages(ctx, state);
    let missing = supplied
        .into_iter()
        .filter(|id| {
            !messages
                .iter()
                .any(|message| message.message_id.as_str() == id.as_str())
        })
        .collect::<Vec<_>>();
    (!missing.is_empty()).then_some(missing)
}

fn relation_asserted_by_person(
    args: &Value,
    ctx: &SessionContext,
    state: &SessionState,
    source_kind: &RelationSource,
) -> Option<PersonId> {
    if let Some(person) = args["asserted_by_person_id"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| PersonId(id.to_string()))
    {
        return Some(person);
    }
    if !matches!(
        source_kind,
        RelationSource::Stated | RelationSource::ChosenPersonConfirmed
    ) {
        return None;
    }
    let evidence_ids = relation_evidence_message_ids(args, ctx, state);
    let messages = evidence_source_messages(ctx, state);
    evidence_ids
        .iter()
        .find_map(|id| messages.iter().find(|message| message.message_id == *id))
        .or_else(|| messages.first())
        .and_then(|message| message.person.clone())
}

fn evidence_source_messages(ctx: &SessionContext, state: &SessionState) -> Vec<InboundMessage> {
    ctx.messages
        .iter()
        .chain(state.presented_injected_messages.iter())
        .chain(state.presented_read_messages.iter())
        .cloned()
        .collect()
}

fn string_array(value: &Value) -> impl Iterator<Item = String> + '_ {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::trim))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::action::{ActionId, ActionKind, RunningState};
    use crate::core::handle::{SharedState, StateHandle};
    use crate::core::tools::SessionKind;
    use crate::state::{ActorState, Authority, Delta, GrowthConfig};
    use crate::store::{SqliteStore, Store};
    use async_trait::async_trait;
    use gateway::GatewayRouter;
    use inference::{
        Capability, ChatRequest, ChatResponse, ChatStream, FinishReason, InferenceEndpoint,
        InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge, Reasoning,
        SamplingConfig, Usage,
    };
    use protocol::{ConversationId, InboundMessage};
    use std::sync::{Arc, RwLock};
    use tokio::sync::mpsc;

    struct NoopBridge;

    #[async_trait]
    impl OpenAiCompatibleBridge for NoopBridge {
        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
            Ok(ChatResponse {
                message: inference::AssistantMessage {
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
            anyhow::bail!("noop bridge is not used by social relation tests")
        }
    }

    fn test_context(store: Arc<SqliteStore>) -> (SessionContext, SessionState) {
        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (state_tx, _state_rx) = mpsc::channel(1);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
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

        let state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: Delta::default(),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        (
            SessionContext {
                action_id: ActionId("social-relation-test".into()),
                kind: SessionKind::Action(ActionKind::Review),
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
                    person: None,
                    content: "Alice is Sam's coworker".into(),
                    attachments: vec![],
                    timestamp: 1000,
                    metadata: Value::Null,
                }],
                conversation: Some(ConversationId("relay:local".into())),
                authority: Authority::Default,
                style_directive: None,
                cancelled_note: None,
                concurrent_summaries: vec![],
                state: StateHandle::new(shared, state_tx),
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
            },
            state,
        )
    }

    #[tokio::test]
    async fn upsert_social_relation_rejects_unavailable_explicit_evidence_message_id() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let (ctx, state) = test_context(store.clone());

        let result = upsert_social_relation(
            &json!({
                "person_a": "person-alice",
                "person_b": "person-sam",
                "relation": "coworker",
                "direction": "bidirectional",
                "confidence": 0.8,
                "status": "stated",
                "source_kind": "stated",
                "evidence_message_ids": ["msg-missing"],
                "evidence_quote": "Sam said Alice is my coworker"
            }),
            &ctx,
            &state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["status"], "error");
        assert_eq!(parsed["missing_evidence_message_ids"][0], "msg-missing");
        let relations = store
            .get_relations(&PersonId("person-alice".into()))
            .await
            .unwrap();
        assert!(relations.is_empty());
    }

    #[tokio::test]
    async fn upsert_social_relation_persists_review_evidence() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let (mut ctx, mut state) = test_context(store.clone());
        let mut second = ctx.messages[0].clone();
        second.message_id = "msg-2".into();
        second.content = "Sam said Alice is my coworker.".into();
        second.timestamp = 1001;
        second.person = Some(PersonId("person-sam".into()));
        ctx.messages.push(second);

        let result = upsert_social_relation(
            &json!({
                "person_a": "person-alice",
                "person_b": "person-sam",
                "relation": "coworker",
                "confidence": 0.8,
                "status": "stated",
                "source_kind": "stated",
                "evidence_message_ids": ["msg-2"],
                "evidence_quote": "Sam said Alice is my coworker",
                "evidence": {"reason": "user stated relationship"}
            }),
            &ctx,
            &state,
        )
        .await;

        assert!(result.contains("\"status\":\"updated\""));
        let relations = store
            .get_relations(&PersonId("person-alice".into()))
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].person_b, PersonId("person-sam".into()));
        assert_eq!(relations[0].relation.as_str(), "coworker");
        assert_eq!(relations[0].direction.as_str(), "bidirectional");
        assert_eq!(relations[0].confidence, 0.8);
        assert_eq!(relations[0].status, RelationStatus::Stated);
        assert_eq!(relations[0].source_kind, RelationSource::Stated);
        assert_eq!(
            relations[0].asserted_by.as_ref(),
            Some(&PersonId("person-sam".into()))
        );
        let evidence = relations[0].evidence.as_ref().unwrap();
        assert_eq!(evidence["message_ids"].as_array().unwrap().len(), 1);
        assert_eq!(evidence["message_ids"][0], "msg-2");
        assert_eq!(evidence["quote"], "Sam said Alice is my coworker");
        assert_eq!(evidence["evidence"]["reason"], "user stated relationship");

        let mut injected = ctx.messages[0].clone();
        injected.message_id = "msg-injected".into();
        injected.content = "Bob said Alice is a friend.".into();
        injected.person = Some(PersonId("person-bob".into()));
        injected.timestamp = 1002;
        state.presented_injected_messages.push(injected);

        let result = upsert_social_relation(
            &json!({
                "person_a": "person-alice",
                "person_b": "person-bob",
                "relation": "friend",
                "confidence": 0.7,
                "status": "stated",
                "source_kind": "stated",
                "evidence_message_ids": ["msg-injected"],
                "evidence_quote": "Bob said Alice is a friend"
            }),
            &ctx,
            &state,
        )
        .await;

        assert!(result.contains("\"status\":\"updated\""));
        let relations = store
            .get_relations(&PersonId("person-bob".into()))
            .await
            .unwrap();
        let relation = relations
            .iter()
            .find(|relation| relation.relation.as_str() == "friend")
            .expect("injected relation persisted");
        assert_eq!(
            relation.asserted_by.as_ref(),
            Some(&PersonId("person-bob".into()))
        );
        let evidence = relation.evidence.as_ref().unwrap();
        assert_eq!(evidence["message_ids"][0], "msg-injected");
        assert_eq!(evidence["quote"], "Bob said Alice is a friend");
    }
}
