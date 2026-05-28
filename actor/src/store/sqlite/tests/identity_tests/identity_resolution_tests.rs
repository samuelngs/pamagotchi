use super::*;

#[tokio::test]
async fn persons_crud() {
    let store = test_store();
    store
        .add_person(&sample_person("p1", "Alice"))
        .await
        .unwrap();
    store.add_person(&sample_person("p2", "Bob")).await.unwrap();

    let alice = store
        .get_person(&PersonId("p1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(alice.name, Some("Alice".into()));

    store
        .update_person(&PersonId("p1".into()), None, Some("likes cats"))
        .await
        .unwrap();
    let alice = store
        .get_person(&PersonId("p1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(alice.summary, Some("likes cats".into()));

    let all = store.list_persons().await.unwrap();
    assert_eq!(all.len(), 2);
}
#[tokio::test]
async fn identity_resolution() {
    let store = test_store();
    store
        .add_person(&sample_person("p1", "Alice"))
        .await
        .unwrap();
    let identity = sample_identity("i1", "discord", "discord-123", "alice#1234");
    let profile = sample_profile("profile-p1", "alice#1234");
    store.add_identity(&identity).await.unwrap();
    store.add_profile(&profile).await.unwrap();
    store
        .link_identity_to_profile(&identity.id, &profile.id, 1.0, None)
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &profile.id,
            &PersonId("p1".into()),
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();

    let found = store
        .resolve_identity("discord", "discord-123")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.identity.id.0, "i1");
    assert_eq!(found.profile.id.0, "profile-p1");
    assert_eq!(found.person.unwrap().id.0, "p1");

    let not_found = store.resolve_identity("telegram", "unknown").await.unwrap();
    assert!(not_found.is_none());

    let identities = store
        .get_identities_for_person(&PersonId("p1".into()))
        .await
        .unwrap();
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].display_name.as_deref(), Some("alice#1234"));
}
#[tokio::test]
async fn identity_claims() {
    let store = test_store();
    store
        .add_person(&sample_person("p1", "Alice Discord"))
        .await
        .unwrap();
    store
        .add_person(&sample_person("p2", "Alice Telegram"))
        .await
        .unwrap();

    store
        .create_claim(&IdentityClaim {
            id: "claim-1".into(),
            claimant: PersonId("p2".into()),
            claimed_person: PersonId("p1".into()),
            evidence: ClaimEvidence::SelfDeclaration,
            reason: Some("They said they are Alice from Discord.".into()),
            evidence_json: serde_json::json!({"message_id": "msg-1"}),
            confidence: 0.1,
            status: ClaimStatus::Pending,
            created_at: 1000,
            resolved_at: None,
        })
        .await
        .unwrap();

    let pending = store.get_pending_claims().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "claim-1");
    assert_eq!(
        pending[0].reason.as_deref(),
        Some("They said they are Alice from Discord.")
    );
    assert_eq!(pending[0].evidence_json["message_id"], "msg-1");

    let recent = store
        .get_recent_claims(Some(&PersonId("p2".into())), None, 900)
        .await
        .unwrap();
    assert_eq!(recent.len(), 1);
    let recent = store
        .get_recent_claims(None, Some(&PersonId("p1".into())), 1001)
        .await
        .unwrap();
    assert!(recent.is_empty());

    store
        .resolve_claim("claim-1", &ClaimStatus::Confirmed)
        .await
        .unwrap();
    let pending = store.get_pending_claims().await.unwrap();
    assert_eq!(pending.len(), 0);
}
#[tokio::test]
async fn identity_disclosure_audit_records_lookup_reason_and_outcome() {
    let store = test_store();
    let target = PersonId("person-target".into());
    let requester = PersonId("person-requester".into());
    store
        .record_identity_disclosure(&IdentityDisclosureAudit {
            id: "audit-allowed".into(),
            action_id: "action-1".into(),
            requester_person: Some(requester.clone()),
            target_person: target.clone(),
            reason: "deliver requested follow-up".into(),
            allowed: true,
            identity_count: 2,
            created_at: 1000,
        })
        .await
        .unwrap();
    store
        .record_identity_disclosure(&IdentityDisclosureAudit {
            id: "audit-denied".into(),
            action_id: "action-2".into(),
            requester_person: Some(requester.clone()),
            target_person: target.clone(),
            reason: "untrusted cross-person lookup".into(),
            allowed: false,
            identity_count: 0,
            created_at: 1001,
        })
        .await
        .unwrap();

    let audits = store
        .identity_disclosures_for_person(&target, 10)
        .await
        .unwrap();
    assert_eq!(audits.len(), 2);
    assert_eq!(audits[0].id, "audit-denied");
    assert_eq!(audits[0].requester_person.as_ref(), Some(&requester));
    assert_eq!(audits[0].reason, "untrusted cross-person lookup");
    assert!(!audits[0].allowed);
    assert_eq!(audits[0].identity_count, 0);
    assert_eq!(audits[1].id, "audit-allowed");
    assert!(audits[1].allowed);
    assert_eq!(audits[1].identity_count, 2);
}
