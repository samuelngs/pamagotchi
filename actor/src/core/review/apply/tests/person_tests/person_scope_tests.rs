use super::*;

#[tokio::test]
async fn person_level_review_updates_require_verified_or_strong_likely_link() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let context_profile = ProfileId("profile-context".into());
    let context_person = PersonId("person-context".into());
    let context_conversation = ConversationId("relay:context".into());
    let (ctx, _) = test_context(
        store.clone(),
        &context_profile,
        &context_person,
        &context_conversation,
    );

    for (idx, status, confidence, allowed) in [
        (0, PersonProfileStatus::Verified, 0.1, true),
        (
            1,
            PersonProfileStatus::Likely,
            permission::STRONG_LIKELY_PERSON_LINK_CONFIDENCE,
            true,
        ),
        (
            2,
            PersonProfileStatus::Likely,
            permission::STRONG_LIKELY_PERSON_LINK_CONFIDENCE - 0.01,
            false,
        ),
        (3, PersonProfileStatus::Suspected, 1.0, false),
    ] {
        let profile = ProfileId(format!("profile-link-{idx}"));
        let person = PersonId(format!("person-link-{idx}"));
        store
            .add_profile(&Profile {
                id: profile.clone(),
                display_name: Some(format!("Profile {idx}")),
                summary: None,
                comm_style: None,
                first_seen: 1000,
                last_seen: 1000,
                created_at: 1000,
                updated_at: 1000,
            })
            .await
            .unwrap();
        store
            .add_person(&Person {
                id: person.clone(),
                name: Some(format!("Person {idx}")),
                summary: None,
                comm_style: None,
                first_seen: 1000,
                last_seen: 1000,
            })
            .await
            .unwrap();
        store
            .attach_profile_to_person(&profile, &person, status, confidence, None)
            .await
            .unwrap();

        assert_eq!(
            permission::person_has_verified_or_strong_profile_context(&ctx, &person)
                .await
                .unwrap(),
            allowed
        );
    }
}
#[tokio::test]
async fn apply_review_skips_person_updates_for_weak_likely_profile_link() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-weak".into());
    let person = PersonId("person-weak".into());
    let conversation = ConversationId("relay:weak".into());
    let now = util::now();
    store
        .add_profile(&Profile {
            id: profile.clone(),
            display_name: Some("Weak".into()),
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
        .add_person(&Person {
            id: person.clone(),
            name: Some("Weak".into()),
            summary: None,
            comm_style: None,
            first_seen: now,
            last_seen: now,
        })
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &profile,
            &person,
            PersonProfileStatus::Likely,
            permission::STRONG_LIKELY_PERSON_LINK_CONFIDENCE - 0.1,
            None,
        )
        .await
        .unwrap();
    let (ctx, mut session_state) = test_context(store.clone(), &profile, &person, &conversation);

    let result = apply(
        &json!({
            "person_updates": [{
                "person_id": person.0,
                "summary": "Weak profile link should not promote to person.",
                "comm_style": "Brief."
            }],
            "memories": [{
                "content": "Weak profile link should not write person-scoped memory.",
                "subjects": [{
                    "type": "person",
                    "id": person.0,
                    "role": "about",
                    "confidence": 1.0
                }],
                "dedupe_key": "review:test:weak-person-memory"
            }],
            "relationship_delta": [{
                "person_id": person.0,
                "familiarity_delta": 0.5,
                "trust_delta": 0.5,
                "valence_delta": 0.5,
                "proactive_consent": "allowed",
                "reason": "weak link should not strengthen relationship"
            }],
            "social_relations": [{
                "person_a": person.0,
                "person_b": "person-alice",
                "relation": "coworker",
                "confidence": 0.8,
                "status": "stated",
                "source_kind": "stated"
            }]
        }),
        &ctx,
        &mut session_state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["person_updates"], 0);
    assert_eq!(parsed["memories"], 0);
    assert_eq!(parsed["relationship_deltas"], 0);
    assert_eq!(parsed["social_relations"], 0);
    assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .is_some_and(|message| message.contains("strongly likely"))
    }));
    assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .is_some_and(|message| message.contains("weak person subject"))
    }));
    assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .is_some_and(|message| message.contains("relationship_delta"))
    }));
    assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .is_some_and(|message| message.contains("verified/strong person anchor"))
    }));
    let stored = store.get_person(&person).await.unwrap().unwrap();
    assert!(stored.summary.is_none());
    assert!(stored.comm_style.is_none());
    let memories = store
        .recall(&crate::store::RecallQuery::by_text(
            "Weak profile link should not write person-scoped memory.",
            5,
        ))
        .await
        .unwrap();
    assert!(memories.is_empty());
}
#[tokio::test]
async fn apply_review_skips_profile_and_person_updates_without_evidence_target() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-current".into());
    let person = PersonId("person-current".into());
    let other_profile = ProfileId("profile-other".into());
    let other_person = PersonId("person-other".into());
    let conversation = ConversationId("relay:current".into());
    store
        .add_profile(&Profile {
            id: other_profile.clone(),
            display_name: Some("Other".into()),
            summary: None,
            comm_style: None,
            first_seen: 1000,
            last_seen: 1000,
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();
    store
        .add_person(&Person {
            id: other_person.clone(),
            name: Some("Other".into()),
            summary: None,
            comm_style: None,
            first_seen: 1000,
            last_seen: 1000,
        })
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &other_profile,
            &other_person,
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();
    let (ctx, mut session_state) = test_context(store.clone(), &profile, &person, &conversation);

    let result = apply(
        &json!({
            "profile_updates": [{
                "profile_id": other_profile.0.clone(),
                "summary": "Unrelated profile summary.",
                "evidence_message_ids": ["msg-1"]
            }],
            "person_updates": [{
                "person_id": other_person.0.clone(),
                "summary": "Unrelated person summary.",
                "evidence_message_ids": ["msg-1"]
            }]
        }),
        &ctx,
        &mut session_state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["profile_updates"], 0);
    assert_eq!(parsed["person_updates"], 0);
    assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .is_some_and(|message| message.contains("profile profile-other is not present"))
    }));
    assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .is_some_and(|message| message.contains("person person-other is not present"))
    }));
    assert!(
        store
            .get_profile(&other_profile)
            .await
            .unwrap()
            .unwrap()
            .summary
            .is_none()
    );
    assert!(
        store
            .get_person(&other_person)
            .await
            .unwrap()
            .unwrap()
            .summary
            .is_none()
    );
}
