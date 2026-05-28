use super::*;

#[tokio::test]
async fn default_user_can_forget_current_profile_memory_only() {
    let mut ctx = test_context(Authority::Default, ActionKind::Respond);
    let profile = ProfileId("profile-current".into());
    ctx.messages[0].profile = Some(profile.clone());

    ctx.store
        .store_memory(&Memory {
            id: MemoryId("memory-current".into()),
            kind: MemoryKind::Semantic,
            content: "current profile preference".into(),
            source: MemorySource::Conversation {
                conversation_id: ctx.messages[0].conversation.clone(),
                identity_id: None,
                profile_id: Some(profile.clone()),
                person_id: None,
                message_id: Some(ctx.messages[0].message_id.clone()),
            },
            subjects: vec![MemorySubject::profile(profile, None, 1.0)],
            ..Memory::default()
        })
        .await
        .unwrap();

    check(
        "forget_memory",
        &serde_json::json!({"memory_id": "memory-current"}),
        &ctx,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn default_user_cannot_forget_other_profile_memory() {
    let ctx = test_context(Authority::Default, ActionKind::Respond);

    ctx.store
        .store_memory(&Memory {
            id: MemoryId("memory-other".into()),
            kind: MemoryKind::Semantic,
            content: "other profile preference".into(),
            source: MemorySource::Conversation {
                conversation_id: ConversationId("relay:other".into()),
                identity_id: None,
                profile_id: Some(ProfileId("profile-other".into())),
                person_id: None,
                message_id: Some("msg-other".into()),
            },
            subjects: vec![MemorySubject::profile(
                ProfileId("profile-other".into()),
                None,
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();

    let denied = check(
        "forget_memory",
        &serde_json::json!({"memory_id": "memory-other"}),
        &ctx,
    )
    .await
    .unwrap_err();

    assert!(denied.contains("outside the current profile"));
}
#[tokio::test]
async fn default_user_cannot_forget_current_person_level_memory() {
    let mut ctx = test_context(Authority::Default, ActionKind::Respond);
    let profile = ProfileId("profile-current".into());
    let person = PersonId("person-current".into());
    ctx.messages[0].profile = Some(profile.clone());
    ctx.messages[0].person = Some(person.clone());

    ctx.store
        .store_memory(&Memory {
            id: MemoryId("memory-person-level".into()),
            kind: MemoryKind::Semantic,
            content: "person-level preference".into(),
            source: MemorySource::Conversation {
                conversation_id: ctx.messages[0].conversation.clone(),
                identity_id: None,
                profile_id: Some(profile),
                person_id: Some(person.clone()),
                message_id: Some(ctx.messages[0].message_id.clone()),
            },
            subjects: vec![MemorySubject::person(person, None, 1.0)],
            visibility_scope: VisibilityScope::Person,
            ..Memory::default()
        })
        .await
        .unwrap();

    let denied = check(
        "forget_memory",
        &serde_json::json!({"memory_id": "memory-person-level"}),
        &ctx,
    )
    .await
    .unwrap_err();

    assert!(denied.contains("outside the current profile"));
}
#[tokio::test]
async fn live_user_cannot_promote_profile_memory_to_person_level_memory() {
    let ctx = test_context(Authority::Default, ActionKind::Respond);

    let denied = check(
        "promote_profile_memory_to_person",
        &serde_json::json!({
            "memory_id": "memory-current-profile",
            "person": "person-current"
        }),
        &ctx,
    )
    .await
    .unwrap_err();

    assert!(denied.contains("Promoting profile memories"));
}
#[tokio::test]
async fn review_can_promote_profile_memory_to_verified_person() {
    let ctx = test_context(Authority::Default, ActionKind::Review);
    let profile = ProfileId("profile-current".into());
    let person = PersonId("person-current".into());
    add_verified_target(&ctx, &profile, &person).await;
    ctx.store
        .store_memory(&Memory {
            id: MemoryId("memory-current-profile".into()),
            kind: MemoryKind::Semantic,
            content: "current profile preference".into(),
            source: MemorySource::Reflection,
            subjects: vec![MemorySubject::profile(profile, Some("about".into()), 1.0)],
            ..Memory::default()
        })
        .await
        .unwrap();

    check(
        "promote_profile_memory_to_person",
        &serde_json::json!({
            "memory_id": "memory-current-profile",
            "person": "person-current"
        }),
        &ctx,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn review_cannot_promote_profile_memory_to_unverified_person() {
    let ctx = test_context(Authority::Default, ActionKind::Review);
    let profile = ProfileId("profile-unverified".into());
    let person = PersonId("person-unverified".into());
    ctx.store
        .add_profile(&Profile {
            id: profile.clone(),
            display_name: Some("Unverified profile".into()),
            summary: None,
            comm_style: None,
            first_seen: 1000,
            last_seen: 1000,
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();
    ctx.store
        .add_person(&Person {
            id: person,
            name: Some("Unverified person".into()),
            summary: None,
            comm_style: None,
            first_seen: 1000,
            last_seen: 1000,
        })
        .await
        .unwrap();
    ctx.store
        .store_memory(&Memory {
            id: MemoryId("memory-unverified-profile".into()),
            kind: MemoryKind::Semantic,
            content: "unverified profile preference".into(),
            source: MemorySource::Reflection,
            subjects: vec![MemorySubject::profile(profile, Some("about".into()), 1.0)],
            ..Memory::default()
        })
        .await
        .unwrap();

    let denied = check(
        "promote_profile_memory_to_person",
        &serde_json::json!({
            "memory_id": "memory-unverified-profile",
            "person": "person-unverified"
        }),
        &ctx,
    )
    .await
    .unwrap_err();

    assert!(denied.contains("verified or strong likely link"));
}
#[tokio::test]
async fn live_user_cannot_demote_person_level_memory_subjects() {
    let ctx = test_context(Authority::Default, ActionKind::Respond);

    let denied = check(
        "demote_person_memory_to_profile",
        &serde_json::json!({
            "memory_id": "memory-person-level",
            "profile": "profile-current"
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("Demoting person-level memories"));

    let review = test_context(Authority::Default, ActionKind::Review);
    check(
        "demote_person_memory_to_profile",
        &serde_json::json!({
            "memory_id": "memory-person-level",
            "profile": "profile-current"
        }),
        &review,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn default_user_cannot_forget_sensitive_current_profile_memory() {
    let mut ctx = test_context(Authority::Default, ActionKind::Respond);
    let profile = ProfileId("profile-current".into());
    ctx.messages[0].profile = Some(profile.clone());

    ctx.store
        .store_memory(&Memory {
            id: MemoryId("memory-sensitive-profile".into()),
            kind: MemoryKind::Semantic,
            content: "current profile sensitive detail".into(),
            source: MemorySource::Conversation {
                conversation_id: ctx.messages[0].conversation.clone(),
                identity_id: None,
                profile_id: Some(profile.clone()),
                person_id: None,
                message_id: Some(ctx.messages[0].message_id.clone()),
            },
            subjects: vec![MemorySubject::profile(profile, None, 1.0)],
            privacy_category: PrivacyCategory::Sensitive,
            ..Memory::default()
        })
        .await
        .unwrap();

    let denied = check(
        "forget_memory",
        &serde_json::json!({"memory_id": "memory-sensitive-profile"}),
        &ctx,
    )
    .await
    .unwrap_err();

    assert!(denied.contains("outside the current profile"));
}
#[tokio::test]
async fn memory_inspection_and_deletion_by_id_are_chosen_person_only() {
    let default = test_context(Authority::Default, ActionKind::Respond);
    for tool in ["inspect_memory", "delete_memory"] {
        let denied = check(
            tool,
            &serde_json::json!({"memory_id": "memory-secret"}),
            &default,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("Chosen-person authority"));
    }

    let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
    for tool in ["inspect_memory", "delete_memory"] {
        check(
            tool,
            &serde_json::json!({"memory_id": "memory-secret"}),
            &chosen_person,
        )
        .await
        .unwrap();
    }
}
