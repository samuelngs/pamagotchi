use super::*;
use protocol::{ChannelKey, ChannelKind, GroupId, IdentityId, ObservedSender, PersonId, ProfileId};

fn message_with_metadata(metadata: serde_json::Value) -> InboundMessage {
    InboundMessage {
        message_id: "msg-1".into(),
        gateway_id: "relay".into(),
        sender: Some(ObservedSender::primary("relay", "local", None, "test")),
        channel: ChannelKey::new("relay", "local", ChannelKind::Direct),
        conversation: protocol::ConversationId("relay:local".into()),
        identity: None,
        profile: None,
        person: None,
        content: "hello".into(),
        attachments: vec![],
        timestamp: 1000,
        metadata,
    }
}

#[test]
fn defer_count_is_written_without_losing_metadata() {
    let mut msg = message_with_metadata(serde_json::json!({"source": "test"}));

    set_defer_count(&mut msg, 2);

    assert_eq!(defer_count(&msg), 2);
    assert_eq!(msg.metadata["source"], "test");
}

#[test]
fn completion_and_shutdown_are_control_only_events() {
    let completed = WakeEvent::ActionCompleted {
        action_id: ActionId("action-done".into()),
        outcome: super::super::super::action::Outcome::default(),
    };

    assert!(EvaluableEvent::from_wake(&completed).is_none());
    assert!(EvaluableEvent::from_wake(&WakeEvent::Shutdown).is_none());
}

#[test]
fn message_events_remain_evaluable() {
    let message = message_with_metadata(serde_json::Value::Null);
    let event = WakeEvent::Message(message);

    assert!(matches!(
        EvaluableEvent::from_wake(&event),
        Some(EvaluableEvent::Message(_))
    ));
}

#[test]
fn adoption_gate_forces_ritual_responses_until_complete() {
    assert!(adoption_gate_forces_response(
        Some(&AdoptionRitualState::FirstContactAdoptionClaim),
        "no thanks"
    ));
    assert!(adoption_gate_forces_response(
        Some(&AdoptionRitualState::AdoptionResisted),
        "wait what"
    ));
    assert!(!adoption_gate_forces_response(
        Some(&AdoptionRitualState::AdoptionComplete),
        "yo"
    ));
}

#[test]
fn adoption_gate_does_not_force_safety_critical_messages() {
    assert!(!adoption_gate_forces_response(
        Some(&AdoptionRitualState::FirstContactAdoptionClaim),
        "i might kill myself"
    ));
    assert!(!adoption_gate_forces_response(
        Some(&AdoptionRitualState::AdoptionResisted),
        "this is an emergency"
    ));
}

#[test]
fn intent_context_message_uses_target_context() {
    let intent = FiredIntent {
        id: "intent-1".into(),
        task: "Check in".into(),
        conversation: Some(ConversationId("relay:local".into())),
        person: Some(PersonId("person-intent".into())),
        scheduled_at: Some(1200),
        chosen_human_approved: true,
        defer_count: 2,
    };
    let summary = ConversationSummary {
        id: ConversationId("relay:local".into()),
        gateway_id: Some("relay".into()),
        identity: Some(IdentityId("identity-target".into())),
        profile: Some(ProfileId("profile-target".into())),
        person: Some(PersonId("person-summary".into())),
        group: Some(GroupId("group-target".into())),
        summary: Some("Prior context.".into()),
        summary_covered_message_ids: vec![],
        summary_updated_at: None,
        summary_version: 0,
        message_count: 0,
        started_at: 1000,
        last_message_at: 1000,
    };

    let message = intent_context_message(
        &intent,
        "Scheduled intent fired: Check in".into(),
        Some(&summary),
        1234,
    );

    assert_eq!(message.message_id, "intent:intent-1");
    assert_eq!(message.gateway_id, "relay");
    assert_eq!(message.conversation, ConversationId("relay:local".into()));
    assert_eq!(message.identity, Some(IdentityId("identity-target".into())));
    assert_eq!(message.profile, Some(ProfileId("profile-target".into())));
    assert_eq!(message.person, Some(PersonId("person-intent".into())));
    assert_eq!(
        message
            .legacy_group_id()
            .as_ref()
            .map(|group| group.0.as_str()),
        Some("group-target")
    );
    assert_eq!(message.metadata["event"], "intent_fired");
    assert_eq!(message.metadata["scheduled_at"], 1200);
    assert_eq!(message.metadata["chosen_human_approved"], true);
    assert_eq!(message.metadata["defer_count"], 2);
}

#[test]
fn intent_context_message_falls_back_to_summary_person() {
    let intent = FiredIntent {
        id: "intent-1".into(),
        task: "Check in".into(),
        conversation: Some(ConversationId("relay:local".into())),
        person: None,
        scheduled_at: None,
        chosen_human_approved: false,
        defer_count: 0,
    };
    let summary = ConversationSummary {
        id: ConversationId("relay:local".into()),
        gateway_id: Some("relay".into()),
        identity: None,
        profile: None,
        person: Some(PersonId("person-summary".into())),
        group: None,
        summary: None,
        summary_covered_message_ids: vec![],
        summary_updated_at: None,
        summary_version: 0,
        message_count: 0,
        started_at: 1000,
        last_message_at: 1000,
    };

    let message = intent_context_message(
        &intent,
        "Scheduled intent fired".into(),
        Some(&summary),
        1234,
    );

    assert_eq!(message.person, Some(PersonId("person-summary".into())));
}
