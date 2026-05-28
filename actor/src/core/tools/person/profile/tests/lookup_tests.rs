use super::*;

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
        authority: Authority::ChosenPerson,
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
