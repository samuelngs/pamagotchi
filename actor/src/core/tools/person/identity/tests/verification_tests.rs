use super::*;

#[tokio::test]
async fn identity_verification_requires_reason_before_contacting_others() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    store.add_person(&person("claimant")).await.unwrap();
    store.add_person(&person("claimed")).await.unwrap();
    let ctx = test_context(store, PersonId("claimant".into()));

    let result = request_identity_verification(
        &json!({
            "claimed_person": "claimed"
        }),
        &ctx,
    )
    .await;
    let value: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(value["status"], "error");
    assert!(value["message"].as_str().unwrap().contains("reason"));
}
#[tokio::test]
async fn identity_verification_requires_recent_explicit_claim_message() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let claimant = PersonId("claimant".into());
    let claimed = PersonId("claimed".into());
    store.add_person(&person("claimant")).await.unwrap();
    store.add_person(&person("claimed")).await.unwrap();
    let mut ctx = test_context(store.clone(), claimant.clone());
    ctx.messages[0].content = "what did claimed say yesterday?".into();

    let result = request_identity_verification(
        &json!({
            "claimed_person": claimed.0,
            "reason": "the model inferred a possible identity link, but the current message did not claim one"
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
            .contains("explicit identity claim")
    );
    let claims = store
        .get_recent_claims(Some(&claimant), Some(&claimed), 0)
        .await
        .unwrap();
    assert!(claims.is_empty());
}
#[tokio::test]
async fn identity_verification_is_rate_limited_for_recent_pair() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    store.add_person(&person("claimant")).await.unwrap();
    store.add_person(&person("claimed")).await.unwrap();
    store
        .create_claim(&IdentityClaim {
            id: "claim-existing".into(),
            claimant: PersonId("claimant".into()),
            claimed_person: PersonId("claimed".into()),
            evidence: ClaimEvidence::SelfDeclaration,
            reason: Some("existing claim".into()),
            evidence_json: json!({"message_id": "old-msg"}),
            confidence: 0.05,
            status: ClaimStatus::Pending,
            created_at: crate::core::tools::util::now(),
            resolved_at: None,
        })
        .await
        .unwrap();
    let ctx = test_context(store, PersonId("claimant".into()));

    let result = request_identity_verification(
        &json!({
            "claimed_person": "claimed",
            "reason": "they said they are the same person"
        }),
        &ctx,
    )
    .await;
    let value: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(value["status"], "rate_limited");
    assert_eq!(value["claim"], "claim-existing");
}
