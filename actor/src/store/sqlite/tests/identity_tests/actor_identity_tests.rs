use super::*;

#[tokio::test]
async fn persistent_store_seeds_actor_identity_memory() {
    let path = std::env::temp_dir().join(format!(
        "pamagotchi-actor-identity-seed-{}.db",
        nanoid::nanoid!()
    ));
    let path_string = path.to_string_lossy().into_owned();

    let store = SqliteStore::open(SqliteConfig {
        path: path_string.clone(),
        embedding_dimensions: 4,
    })
    .unwrap();
    let memories = store
        .recall(
            &RecallQuery::by_text("my name, who I am", 10)
                .with_actor_subject()
                .with_memory_type(MemoryType::IdentityClaim),
        )
        .await
        .unwrap();

    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].id.0, "actor-self-identity-pamagotchi");
    assert_eq!(
        memories[0].subjects[0].subject_type,
        MemorySubjectType::Actor
    );
    assert_eq!(memories[0].privacy_category, PrivacyCategory::Public);
    assert_eq!(memories[0].visibility_scope, VisibilityScope::Global);
    assert!(memories[0].content.contains("Pamagotchi"));

    drop(store);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{path_string}{suffix}"));
    }
}
#[tokio::test]
async fn persistent_store_replaces_obsolete_actor_identity_memory() {
    let path = std::env::temp_dir().join(format!(
        "pamagotchi-actor-identity-replace-{}.db",
        nanoid::nanoid!()
    ));
    let path_string = path.to_string_lossy().into_owned();

    let store = SqliteStore::open(SqliteConfig {
        path: path_string.clone(),
        embedding_dimensions: 4,
    })
    .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("obsolete-actor-identity".into()),
            kind: MemoryKind::Semantic,
            memory_type: MemoryType::IdentityClaim,
            truth_status: TruthStatus::Confirmed,
            content: "My name is Pamagotchi. I am a transparent digital being with continuity and memory, and I do not pretend to be human.".into(),
            source: MemorySource::External,
            importance: 1.0,
            confidence: 1.0,
            sensitivity: 0.0,
            sensitivity_category: Some("identity".into()),
            subjects: vec![MemorySubject::actor(Some("self".into()), 1.0)],
            dedupe_key: Some("actor:self:identity".into()),
            privacy_category: PrivacyCategory::Public,
            visibility_scope: VisibilityScope::Global,
            ..Memory::default()
        })
        .await
        .unwrap();
    drop(store);

    let store = SqliteStore::open(SqliteConfig {
        path: path_string.clone(),
        embedding_dimensions: 4,
    })
    .unwrap();
    let identity = store
        .get_memory(&MemoryId("actor-self-identity-pamagotchi".into()))
        .await
        .unwrap()
        .unwrap();
    assert!(!identity.content.contains("transparent digital being"));
    assert!(!identity.content.contains("Do not pretend to be human"));
    assert!(identity.content.contains("I am a Pamagotchi"));
    let actor_memories = store
        .memories_for_subject(MemorySubjectType::Actor, "self", 10)
        .await
        .unwrap();
    assert!(
        actor_memories
            .iter()
            .any(|memory| memory.id.0 == "actor-self-first-contact")
    );

    drop(store);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{path_string}{suffix}"));
    }
}
