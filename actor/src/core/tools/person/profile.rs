use super::super::context::SessionContext;
use super::helpers::{current_profile, resolve_person_ref};
use crate::state::Authority;
use crate::store::IdentityDisclosureAudit;
use protocol::{PersonId, ProfileId};
use serde_json::{Value, json};
use tracing::{info, warn};

pub async fn update(args: &Value, ctx: &SessionContext) -> String {
    let person_id = resolve_person_ref(args, ctx);
    let Some(person_id) = person_id else {
        return "No person ref provided and no current conversation partner.".into();
    };

    let name = args["name"].as_str();
    let summary = args["summary"].as_str();
    let comm_style = args["comm_style"].as_str();

    if name.is_none() && summary.is_none() && comm_style.is_none() {
        return "Nothing to update — provide name, summary, or comm_style.".into();
    }

    if name.is_some() || summary.is_some() {
        if let Err(e) = ctx.store.update_person(&person_id, name, summary).await {
            return json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string();
        }
    }
    if let Some(style) = comm_style {
        if let Err(e) = ctx.store.update_comm_style(&person_id, style).await {
            return json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string();
        }
    }

    info!(action = %ctx.action_id, person = %person_id.0, "person updated");
    let mut parts = Vec::new();
    if name.is_some() {
        parts.push("name");
    }
    if summary.is_some() {
        parts.push("summary");
    }
    if comm_style.is_some() {
        parts.push("comm_style");
    }
    json!({
        "status": "updated",
        "ref": person_id.0,
        "fields": parts,
    })
    .to_string()
}

pub async fn update_profile(args: &Value, ctx: &SessionContext) -> String {
    let profile_id = args["ref"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|id| ProfileId(id.to_string()))
        .or_else(|| current_profile(ctx));
    let Some(profile_id) = profile_id else {
        return "No profile ref provided and no current profile.".into();
    };

    let display_name = args["display_name"].as_str();
    let summary = args["summary"].as_str();
    let comm_style = args["comm_style"].as_str();

    if display_name.is_none() && summary.is_none() && comm_style.is_none() {
        return "Nothing to update — provide display_name, summary, or comm_style.".into();
    }

    if display_name.is_some() || summary.is_some() {
        if let Err(e) = ctx
            .store
            .update_profile(&profile_id, display_name, summary)
            .await
        {
            return json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string();
        }
    }
    if let Some(style) = comm_style {
        if let Err(e) = ctx
            .store
            .update_profile_comm_style(&profile_id, style)
            .await
        {
            return json!({
            "status": "error",
            "message": format!("{e}"),
            })
            .to_string();
        }
    }

    info!(action = %ctx.action_id, profile = %profile_id.0, "profile updated");
    let mut parts = Vec::new();
    if display_name.is_some() {
        parts.push("display_name");
    }
    if summary.is_some() {
        parts.push("summary");
    }
    if comm_style.is_some() {
        parts.push("comm_style");
    }
    json!({
        "status": "updated",
        "ref": profile_id.0,
        "fields": parts,
    })
    .to_string()
}

pub async fn get(args: &Value, ctx: &SessionContext) -> String {
    let person_id = resolve_person_ref(args, ctx);
    let Some(person_id) = person_id else {
        return json!({
            "status": "error",
            "message": "No person ref provided and no current conversation partner.",
        })
        .to_string();
    };
    let include_identities = args["include_identities"].as_bool().unwrap_or(false);
    let delivery_required = args["delivery_required"].as_bool().unwrap_or(false);
    let identity_reason = match identity_lookup_reason(args) {
        Ok(reason) => reason,
        Err(message) => {
            return json!({
                "status": "error",
                "message": message,
            })
            .to_string();
        }
    };

    match ctx.store.get_person(&person_id).await {
        Ok(Some(person)) => {
            let first_seen = chrono::DateTime::from_timestamp(person.first_seen, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| person.first_seen.to_string());
            let last_seen = chrono::DateTime::from_timestamp(person.last_seen, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| person.last_seen.to_string());

            let mut response = json!({
                "ref": person.id.0,
                "name": person.name,
                "summary": person.summary,
                "comm_style": person.comm_style,
                "first_seen": first_seen,
                "last_seen": last_seen,
            });

            if include_identities {
                let reason = identity_reason.expect("validated reason exists");
                info!(
                    action = %ctx.action_id,
                    person = %person_id.0,
                    reason,
                    "person identities requested"
                );
                match visible_identities(&person_id, ctx, delivery_required).await {
                    Ok(identities) => {
                        let identity_count = identities.as_array().map_or(0, |items| items.len());
                        match record_identity_disclosure(
                            ctx,
                            &person_id,
                            reason,
                            true,
                            identity_count as u32,
                        )
                        .await
                        {
                            Ok(()) => response["identities"] = identities,
                            Err(e) => {
                                warn!(
                                    action = %ctx.action_id,
                                    person = %person_id.0,
                                    %e,
                                    "refusing identity disclosure because audit failed"
                                );
                                response["identities_error"] = json!(
                                    "Identity lookup could not be audited, so identities were not returned."
                                );
                            }
                        }
                    }
                    Err(message) => {
                        if let Err(e) =
                            record_identity_disclosure(ctx, &person_id, reason, false, 0).await
                        {
                            warn!(
                                action = %ctx.action_id,
                                person = %person_id.0,
                                %e,
                                "failed to audit denied identity disclosure"
                            );
                        }
                        response["identities_error"] = json!(message);
                    }
                }
            }

            response.to_string()
        }
        Ok(None) => json!({
            "status": "error",
            "message": "Person not found.",
        })
        .to_string(),
        Err(e) => json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string(),
    }
}

async fn record_identity_disclosure(
    ctx: &SessionContext,
    target_person: &PersonId,
    reason: &str,
    allowed: bool,
    identity_count: u32,
) -> anyhow::Result<()> {
    let audit = IdentityDisclosureAudit {
        id: format!("identity-disclosure-{}", super::super::util::uuid_v4()),
        action_id: ctx.action_id.0.clone(),
        requester_person: ctx
            .messages
            .first()
            .and_then(|message| message.person.clone()),
        target_person: target_person.clone(),
        reason: reason.to_string(),
        allowed,
        identity_count,
        created_at: super::super::util::now(),
    };
    ctx.store.record_identity_disclosure(&audit).await
}

fn identity_lookup_reason(args: &Value) -> Result<Option<&str>, &'static str> {
    if !args["include_identities"].as_bool().unwrap_or(false) {
        return Ok(None);
    }

    args["reason"]
        .as_str()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .map(Some)
        .ok_or("Provide reason when include_identities=true so the identity lookup is auditable.")
}

async fn visible_identities(
    person: &PersonId,
    ctx: &SessionContext,
    reveal_external_ids: bool,
) -> Result<Value, String> {
    let current = ctx.messages.first().and_then(|m| m.person.as_ref());
    let is_self = current == Some(person);
    let is_owner = ctx.authority == Authority::Owner;

    if !is_self && !is_owner {
        return Err("Identities are private. If this is an identity claim, use request_identity_verification instead.".into());
    }

    match ctx.store.get_identities_for_person(person).await {
        Ok(identities) => Ok(json!(
            identities
                .into_iter()
                .map(|ident| {
                    let mut item = json!({
                        "id": ident.id.0,
                        "gateway_id": ident.gateway_id,
                        "display_name": ident.display_name,
                    });
                    if reveal_external_ids {
                        item["external_id"] = json!(ident.external_id);
                    } else {
                        item["external_id_masked"] = json!(mask_external_id(&ident.external_id));
                    }
                    item
                })
                .collect::<Vec<_>>()
        )),
        Err(e) => Err(format!("{e}")),
    }
}

fn mask_external_id(external_id: &str) -> String {
    let chars = external_id.chars().collect::<Vec<_>>();
    if chars.len() <= 4 {
        return "*".repeat(chars.len().max(1));
    }
    let tail = chars[chars.len() - 4..].iter().collect::<String>();
    format!("***{tail}")
}

#[cfg(test)]
mod tests {
    use super::{get, identity_lookup_reason, mask_external_id};
    use crate::core::action::{ActionId, ActionKind, RunningState};
    use crate::core::handle::{SharedState, StateHandle};
    use crate::core::tools::{SessionContext, SessionKind};
    use crate::identity::{Identity, Person, PersonProfileStatus, Profile, ProfileIdentityStatus};
    use crate::state::{ActorState, Authority, GrowthConfig};
    use crate::store::{SqliteStore, Store};
    use async_trait::async_trait;
    use gateway::GatewayRouter;
    use inference::{
        Capability, ChatRequest, ChatResponse, ChatStream, FinishReason, InferenceEndpoint,
        InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge, Reasoning,
        SamplingConfig, Usage,
    };
    use protocol::{ConversationId, IdentityId, InboundMessage, PersonId, ProfileId};
    use serde_json::json;
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
            anyhow::bail!("noop bridge is not used by get_person tests")
        }
    }

    #[test]
    fn identity_lookup_requires_reason_when_identities_requested() {
        assert_eq!(identity_lookup_reason(&json!({})).unwrap(), None);
        assert!(
            identity_lookup_reason(&json!({
                "include_identities": true
            }))
            .is_err()
        );
        assert!(
            identity_lookup_reason(&json!({
                "include_identities": true,
                "reason": "   "
            }))
            .is_err()
        );
        assert_eq!(
            identity_lookup_reason(&json!({
                "include_identities": true,
                "reason": "deliver a requested follow-up"
            }))
            .unwrap(),
            Some("deliver a requested follow-up")
        );
    }

    #[test]
    fn external_identity_mask_keeps_only_short_suffix() {
        assert_eq!(mask_external_id("target-ext"), "***-ext");
        assert_eq!(mask_external_id("abc"), "***");
    }

    #[tokio::test]
    async fn get_person_identity_lookup_is_durably_audited() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let requester = PersonId("person-requester".into());
        let target = PersonId("person-target".into());
        let profile = ProfileId("profile-target".into());
        let identity = IdentityId("identity-target".into());
        let now = 1000;

        store
            .add_person(&Person {
                id: requester.clone(),
                name: Some("Requester".into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
            })
            .await
            .unwrap();
        store
            .add_person(&Person {
                id: target.clone(),
                name: Some("Target".into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
            })
            .await
            .unwrap();
        store
            .add_profile(&Profile {
                id: profile.clone(),
                display_name: Some("Target".into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        store
            .add_identity(&Identity {
                id: identity.clone(),
                gateway_id: "discord".into(),
                external_id: "target-ext".into(),
                display_name: Some("target".into()),
                metadata: None,
                created_at: now,
                last_seen_at: now,
            })
            .await
            .unwrap();
        store
            .link_identity_to_profile(
                &identity,
                &profile,
                1.0,
                Some(&json!({
                    "status": ProfileIdentityStatus::Active.as_str()
                })),
            )
            .await
            .unwrap();
        store
            .attach_profile_to_person(
                &profile,
                &target,
                PersonProfileStatus::Verified,
                1.0,
                Some(&json!({"reason": "test"})),
            )
            .await
            .unwrap();

        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
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
        let ctx = SessionContext {
            action_id: ActionId("get-person-test".into()),
            kind: SessionKind::Action(ActionKind::Respond),
            messages: vec![InboundMessage {
                message_id: "msg-1".into(),
                gateway_id: "relay".into(),
                sender_external_id: "local".into(),
                sender_display_name: Some("Requester".into()),
                reply_external_id: "local".into(),
                conversation: ConversationId("relay:local".into()),
                group: None,
                identity: None,
                profile: None,
                person: Some(requester.clone()),
                content: "send them the follow-up".into(),
                attachments: vec![],
                timestamp: now,
                metadata: serde_json::Value::Null,
            }],
            conversation: Some(ConversationId("relay:local".into())),
            authority: Authority::Owner,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: StateHandle::new(shared, delta_tx),
            store: store_dyn,
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
        };

        let result = get(
            &json!({
                "ref": target.0.clone(),
                "include_identities": true,
                "delivery_required": true,
                "reason": "deliver requested follow-up"
            }),
            &ctx,
        )
        .await;
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert!(value.get("identities_error").is_none());
        assert_eq!(value["identities"][0]["external_id"], "target-ext");

        let audits = store
            .identity_disclosures_for_person(&target, 10)
            .await
            .unwrap();
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].action_id, "get-person-test");
        assert_eq!(audits[0].requester_person.as_ref(), Some(&requester));
        assert_eq!(audits[0].reason, "deliver requested follow-up");
        assert!(audits[0].allowed);
        assert_eq!(audits[0].identity_count, 1);
    }

    #[tokio::test]
    async fn get_person_identity_lookup_masks_external_ids_without_delivery_need() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let target = PersonId("person-target".into());
        let profile = ProfileId("profile-target".into());
        let identity = IdentityId("identity-target".into());
        let now = 1000;

        store
            .add_person(&Person {
                id: target.clone(),
                name: Some("Target".into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
            })
            .await
            .unwrap();
        store
            .add_profile(&Profile {
                id: profile.clone(),
                display_name: Some("Target".into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        store
            .add_identity(&Identity {
                id: identity.clone(),
                gateway_id: "discord".into(),
                external_id: "target-ext".into(),
                display_name: Some("target".into()),
                metadata: None,
                created_at: now,
                last_seen_at: now,
            })
            .await
            .unwrap();
        store
            .link_identity_to_profile(&identity, &profile, 1.0, None)
            .await
            .unwrap();
        store
            .attach_profile_to_person(&profile, &target, PersonProfileStatus::Verified, 1.0, None)
            .await
            .unwrap();

        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
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
        let ctx = SessionContext {
            action_id: ActionId("get-person-mask-test".into()),
            kind: SessionKind::Action(ActionKind::Respond),
            messages: vec![InboundMessage {
                message_id: "msg-1".into(),
                gateway_id: "relay".into(),
                sender_external_id: "local".into(),
                sender_display_name: Some("Target".into()),
                reply_external_id: "local".into(),
                conversation: ConversationId("relay:local".into()),
                group: None,
                identity: None,
                profile: None,
                person: Some(target.clone()),
                content: "what accounts are linked?".into(),
                attachments: vec![],
                timestamp: now,
                metadata: serde_json::Value::Null,
            }],
            conversation: Some(ConversationId("relay:local".into())),
            authority: Authority::Default,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: StateHandle::new(shared, delta_tx),
            store: store_dyn,
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
        };

        let result = get(
            &json!({
                "include_identities": true,
                "reason": "inspect linked accounts"
            }),
            &ctx,
        )
        .await;
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert!(value["identities"][0].get("external_id").is_none());
        assert_eq!(value["identities"][0]["external_id_masked"], "***-ext");
    }
}
