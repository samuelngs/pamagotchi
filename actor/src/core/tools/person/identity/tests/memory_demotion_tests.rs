use super::*;

#[tokio::test]
async fn denied_identity_verification_demotes_promoted_profile_memories() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let claimant = PersonId("claimant".into());
    let claimed = PersonId("claimed".into());
    let claimant_profile = ProfileId("profile-claimant".into());
    store.add_person(&person("claimant")).await.unwrap();
    store.add_person(&person("claimed")).await.unwrap();
    attach_reachable_identity_to_person(&store, &claimant).await;
    store
        .create_claim(&IdentityClaim {
            id: "claim-denied".into(),
            claimant: claimant.clone(),
            claimed_person: claimed.clone(),
            evidence: ClaimEvidence::SharedKnowledge,
            reason: Some("current profile claimed to be the known person".into()),
            evidence_json: json!({"message_id": "msg-claim"}),
            confidence: 0.4,
            status: ClaimStatus::Pending,
            created_at: crate::core::tools::util::now(),
            resolved_at: None,
        })
        .await
        .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("promoted-memory".into()),
            kind: MemoryKind::Semantic,
            content: "Claimant prefers concise replies.".into(),
            source: MemorySource::Reflection,
            subjects: vec![
                MemorySubject::profile(claimant_profile.clone(), Some("about".into()), 1.0),
                MemorySubject::person(claimed.clone(), Some("about".into()), 1.0),
            ],
            ..Memory::default()
        })
        .await
        .unwrap();
    let ctx = test_context(store.clone(), claimed.clone());

    let result = resolve_identity_verification(
        &json!({
            "claim": "claim-denied",
            "confirmed": false
        }),
        &ctx,
    )
    .await;
    let value: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(value["status"], "denied");
    assert_eq!(value["memories_demoted"], 1);

    let memory = store
        .get_memory(&MemoryId("promoted-memory".into()))
        .await
        .unwrap()
        .unwrap();
    assert!(memory.subjects.iter().any(|subject| {
        subject.subject_type == MemorySubjectType::Profile
            && subject.subject_id == claimant_profile.0
    }));
    assert!(!memory.subjects.iter().any(|subject| {
        subject.subject_type == MemorySubjectType::Person && subject.subject_id == claimed.0
    }));
    assert!(memory.content.contains("- profile profile-claimant"));
    assert!(!memory.content.contains("- person claimed"));
}
