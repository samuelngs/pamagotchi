use super::*;

#[test]
fn external_identity_mask_keeps_only_short_suffix() {
    assert_eq!(mask_external_id("target-ext"), "***-ext");
    assert_eq!(mask_external_id("abc"), "***");
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
            sender: Some(protocol::ObservedSender::primary(
                "relay",
                "local",
                Some("Target".into()),
                "test",
            )),
            channel: protocol::ChannelKey::new("relay", "local", protocol::ChannelKind::Direct),
            conversation: ConversationId("relay:local".into()),
            identity: None,
            profile: None,
            person: Some(target.clone()),
            content: "what accounts are linked?".into(),
            attachments: vec![],
            timestamp: now,
            metadata: serde_json::Value::Null,
        }],
        conversation: Some(ConversationId("relay:local".into())),
        relationship_standing: RelationshipStanding::Default,
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
