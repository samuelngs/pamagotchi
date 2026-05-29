use super::*;

#[tokio::test]
async fn self_declaration_identity_claim_records_without_contacting_known_identities() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let claimant = PersonId("claimant".into());
    let claimed = PersonId("claimed".into());
    store.add_person(&person("claimant")).await.unwrap();
    store.add_person(&person("claimed")).await.unwrap();
    attach_reachable_identity_to_person(&store, &claimed).await;
    let ctx = test_context(store.clone(), claimant.clone());

    let result = request_identity_verification(
        &json!({
            "claimed_person": claimed.0,
            "reason": "the current profile said they are the same person on another platform",
            "evidence": "self_declaration"
        }),
        &ctx,
    )
    .await;
    let value: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(value["status"], "evidence_required");
    assert_eq!(value["contacted"], 0);
    assert_eq!(value["failed"], 0);
    assert!(
        value["message"]
            .as_str()
            .unwrap()
            .contains("stronger evidence")
    );
    let claims = store
        .get_recent_claims(Some(&claimant), Some(&claimed), 0)
        .await
        .unwrap();
    assert_eq!(claims.len(), 1);
    assert!(matches!(claims[0].evidence, ClaimEvidence::SelfDeclaration));
    assert_eq!(claims[0].confidence, 0.05);
}
#[tokio::test]
async fn non_chosen_human_cannot_escalate_identity_claim_evidence_to_chosen_human_vouched() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let claimant = PersonId("claimant".into());
    let claimed = PersonId("claimed".into());
    store.add_person(&person("claimant")).await.unwrap();
    store.add_person(&person("claimed")).await.unwrap();
    let ctx = test_context(store.clone(), claimant.clone());

    let result = request_identity_verification(
        &json!({
            "claimed_person": claimed.0,
            "reason": "the current profile claimed chosen human vouched for them",
            "evidence": "chosen_human_vouched"
        }),
        &ctx,
    )
    .await;
    let value: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(value["status"], "error");
    assert!(
        value["message"]
            .as_str()
            .unwrap()
            .contains("require chosen-human relationship standing")
    );
    let claims = store
        .get_recent_claims(Some(&claimant), Some(&claimed), 0)
        .await
        .unwrap();
    assert!(claims.is_empty());
}
#[tokio::test]
async fn default_identity_verification_for_chosen_human_records_claim_without_contacting_chosen_human()
 {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let claimant = PersonId("claimant".into());
    let chosen_human = PersonId("chosen_human".into());
    store.add_person(&person("claimant")).await.unwrap();
    store.add_person(&person("chosen_human")).await.unwrap();
    attach_reachable_identity_to_person(&store, &chosen_human).await;
    let ctx = test_context_with_relationships(
        store.clone(),
        claimant.clone(),
        vec![(chosen_human.clone(), RelationshipStanding::ChosenHuman)],
    );

    let result = request_identity_verification(
        &json!({
            "claimed_person": "chosen_human",
            "reason": "the current profile claimed to be the chosen human on another platform"
        }),
        &ctx,
    )
    .await;
    let value: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(value["status"], "chosen_human_confirmation_required");
    let chosen_human_intent = value["chosen_human_intent"]
        .as_str()
        .expect("chosen-human review intent is created")
        .to_string();
    assert_eq!(value["contacted"], 0);
    assert_eq!(value["failed"], 0);

    let claims = store
        .get_recent_claims(Some(&claimant), Some(&chosen_human), 0)
        .await
        .unwrap();
    assert_eq!(claims.len(), 1);
    assert!(matches!(claims[0].status, ClaimStatus::Pending));
    assert_eq!(
        claims[0].reason.as_deref(),
        Some("the current profile claimed to be the chosen human on another platform")
    );

    let intents = store
        .active_intents_for_context(
            Some(&chosen_human),
            None,
            None,
            crate::core::tools::util::now(),
            10,
        )
        .await
        .unwrap();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].id, chosen_human_intent);
    assert_eq!(intents[0].person.as_ref(), Some(&chosen_human));
    assert_eq!(intents[0].priority, 100);
    assert!(intents[0].chosen_human_approved);
    assert!(intents[0].task.contains(&claims[0].id));
    assert!(intents[0].task.contains("before anyone is contacted"));
}
