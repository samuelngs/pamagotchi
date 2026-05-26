use super::*;
use crate::identity::ClaimEvidence;
use crate::state::{ActorState, CoreTraits, DirectiveScope, GrowthConfig};
use crate::store::{MemoryKind, MemorySource, MessageRole};

fn test_store() -> SqliteStore {
    SqliteStore::open_in_memory(4).unwrap()
}

fn sample_memory(id: &str, content: &str, embedding: Vec<f32>) -> Memory {
    Memory {
        id: MemoryId(id.into()),
        kind: MemoryKind::Episodic,
        content: content.into(),
        source: MemorySource::Conversation {
            conversation_id: ConversationId("conv-1".into()),
            identity_id: None,
            profile_id: Some(ProfileId("profile-sam".into())),
            person_id: Some(PersonId("sam".into())),
            message_id: None,
        },
        importance: 0.8,
        sensitivity: 0.0,
        emotional_valence: -0.3,
        created_at: 1000,
        accessed_at: 1000,
        access_count: 0,
        tags: vec!["work".into()],
        subjects: vec![MemorySubject::profile(
            ProfileId("profile-sam".into()),
            Some("speaker".into()),
            1.0,
        )],
        embedding: Some(embedding),
    }
}

#[tokio::test]
async fn memory_store_and_recall_by_text() {
    let store = test_store();
    let mem = sample_memory(
        "m1",
        "deployment incident was stressful",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    store.store_memory(&mem).await.unwrap();

    let results = store
        .recall(&RecallQuery::by_text("deployment", 10))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "m1");
}

#[tokio::test]
async fn memory_recall_returns_newest_first() {
    let store = test_store();
    let mut older = sample_memory("older", "profile fact changed", vec![0.1, 0.2, 0.3, 0.4]);
    older.created_at = 1000;
    older.accessed_at = 1000;
    let mut newer = sample_memory("newer", "profile fact changed", vec![0.1, 0.2, 0.3, 0.4]);
    newer.created_at = 2000;
    newer.accessed_at = 2000;

    store.store_memory(&older).await.unwrap();
    store.store_memory(&newer).await.unwrap();

    let results = store
        .recall(&RecallQuery::by_text("profile", 10))
        .await
        .unwrap();
    let ids = results.iter().map(|m| m.id.0.as_str()).collect::<Vec<_>>();
    assert_eq!(ids, vec!["newer", "older"]);
}

#[tokio::test]
async fn memory_recall_by_embedding() {
    let store = test_store();
    store
        .store_memory(&sample_memory("m1", "first", vec![1.0, 0.0, 0.0, 0.0]))
        .await
        .unwrap();
    store
        .store_memory(&sample_memory("m2", "second", vec![0.0, 1.0, 0.0, 0.0]))
        .await
        .unwrap();

    let results = store
        .recall(&RecallQuery::by_embedding(vec![0.9, 0.1, 0.0, 0.0], 1))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "m1");
}

#[tokio::test]
async fn memory_get_loads_embedding() {
    let store = test_store();
    store
        .store_memory(&sample_memory("m1", "test", vec![0.1, 0.2, 0.3, 0.4]))
        .await
        .unwrap();

    let loaded = store
        .get_memory(&MemoryId("m1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.embedding.unwrap(), vec![0.1, 0.2, 0.3, 0.4]);
}

#[tokio::test]
async fn memory_forget() {
    let store = test_store();
    store
        .store_memory(&sample_memory("m1", "gone", vec![0.1, 0.2, 0.3, 0.4]))
        .await
        .unwrap();

    assert!(store.forget(&MemoryId("m1".into())).await.unwrap());
    assert!(
        store
            .get_memory(&MemoryId("m1".into()))
            .await
            .unwrap()
            .is_none()
    );
    assert!(!store.forget(&MemoryId("m1".into())).await.unwrap());
}

#[tokio::test]
async fn conversation_messages() {
    let store = test_store();
    let conv = ConversationId("c1".into());

    store
        .append_message(
            &conv,
            None,
            None,
            &StoredMessage {
                timestamp: 1000,
                role: MessageRole::User,
                content: "hello".into(),
                identity: None,
                profile: Some(ProfileId("profile-sam".into())),
                person: Some(PersonId("sam".into())),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();

    store
        .append_message(
            &conv,
            None,
            None,
            &StoredMessage {
                timestamp: 1001,
                role: MessageRole::Assistant,
                content: "hi there".into(),
                identity: None,
                profile: None,
                person: None,
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();

    let msgs = store.get_messages(&conv, 10, None).await.unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].content, "hello");
    assert_eq!(msgs[1].content, "hi there");

    let convs = store.list_conversations().await.unwrap();
    assert_eq!(convs.len(), 1);
    assert_eq!(convs[0].message_count, 2);
}

#[tokio::test]
async fn thoughts() {
    let store = test_store();
    store
        .log_thought(&Thought {
            timestamp: 2000,
            kind: ThoughtKind::Reflection,
            content: "Sam seemed stressed".into(),
            memories_accessed: vec![MemoryId("m1".into())],
            subjects: vec![MemorySubject::profile(
                ProfileId("profile-sam".into()),
                Some("about".into()),
                1.0,
            )],
        })
        .await
        .unwrap();

    let thoughts = store.recent_thoughts(5).await.unwrap();
    assert_eq!(thoughts.len(), 1);
    assert_eq!(thoughts[0].content, "Sam seemed stressed");
    assert_eq!(thoughts[0].subjects[0].subject_id, "profile-sam");
}

#[tokio::test]
async fn snapshots() {
    let store = test_store();
    let snapshot = ActorSnapshot {
        state: ActorState::new(CoreTraits::default()),
        config: GrowthConfig::default(),
        saved_at: 3000,
    };
    store.save_snapshot(&snapshot).await.unwrap();

    let loaded = store.load_latest_snapshot().await.unwrap().unwrap();
    assert_eq!(loaded.saved_at, 3000);
}

#[tokio::test]
async fn recall_filters() {
    let store = test_store();
    store
        .store_memory(&Memory {
            id: MemoryId("m1".into()),
            kind: MemoryKind::Episodic,
            content: "episodic thing".into(),
            source: MemorySource::Reflection,
            importance: 0.9,
            sensitivity: 0.0,
            emotional_valence: 0.0,
            created_at: 1000,
            accessed_at: 1000,
            access_count: 0,
            tags: vec![],
            subjects: vec![],
            embedding: None,
        })
        .await
        .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("m2".into()),
            kind: MemoryKind::Semantic,
            content: "semantic fact".into(),
            source: MemorySource::Reflection,
            importance: 0.3,
            sensitivity: 0.0,
            emotional_valence: 0.0,
            created_at: 1000,
            accessed_at: 1000,
            access_count: 0,
            tags: vec![],
            subjects: vec![],
            embedding: None,
        })
        .await
        .unwrap();

    let results = store
        .recall(&RecallQuery::by_text("thing", 10).with_kind(MemoryKind::Episodic))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "m1");

    let results = store
        .recall(&RecallQuery::by_text("", 10).with_min_importance(0.5))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "m1");
}

fn sample_person(id: &str, name: &str) -> Person {
    Person {
        id: PersonId(id.into()),
        name: Some(name.into()),
        summary: None,
        comm_style: None,
        first_seen: 1000,
        last_seen: 1000,
    }
}

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

fn sample_identity(id: &str, gateway_id: &str, external_id: &str, display_name: &str) -> Identity {
    Identity {
        id: IdentityId(id.into()),
        gateway_id: gateway_id.into(),
        external_id: external_id.into(),
        display_name: Some(display_name.into()),
        metadata: None,
        created_at: 1000,
        last_seen_at: 1000,
    }
}

fn sample_profile(id: &str, display_name: &str) -> Profile {
    Profile {
        id: ProfileId(id.into()),
        display_name: Some(display_name.into()),
        summary: None,
        comm_style: None,
        first_seen: 1000,
        last_seen: 1000,
        created_at: 1000,
        updated_at: 1000,
    }
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

    store
        .resolve_claim("claim-1", &ClaimStatus::Confirmed)
        .await
        .unwrap();
    let pending = store.get_pending_claims().await.unwrap();
    assert_eq!(pending.len(), 0);
}

#[tokio::test]
async fn profile_attach_reconnects_identity_without_deleting_person() {
    let store = test_store();
    store
        .add_person(&sample_person("p1", "Alice"))
        .await
        .unwrap();
    store
        .add_person(&sample_person("p2", "Alice Alt"))
        .await
        .unwrap();

    let identity = sample_identity("i2", "telegram", "tg-alice", "alice_t");
    let profile = sample_profile("profile-p2", "alice_t");
    store.add_identity(&identity).await.unwrap();
    store.add_profile(&profile).await.unwrap();
    store
        .link_identity_to_profile(&identity.id, &profile.id, 1.0, None)
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &profile.id,
            &PersonId("p2".into()),
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();

    let conv = ConversationId("c1".into());
    store
        .append_message(
            &conv,
            None,
            None,
            &StoredMessage {
                timestamp: 1000,
                role: MessageRole::User,
                content: "from alt account".into(),
                identity: Some(identity.id.clone()),
                profile: Some(profile.id.clone()),
                person: Some(PersonId("p2".into())),
                metadata: serde_json::Value::Null,
            },
        )
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

    let resolved = store
        .resolve_identity("telegram", "tg-alice")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.person.unwrap().id.0, "p1");

    assert!(
        store
            .get_person(&PersonId("p2".into()))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn social_graph() {
    let store = test_store();
    store.add_person(&sample_person("p1", "Sam")).await.unwrap();
    store.add_person(&sample_person("p2", "Mom")).await.unwrap();

    store
        .add_relation(
            &PersonId("p2".into()),
            &PersonId("p1".into()),
            &Relation::Parent,
        )
        .await
        .unwrap();

    let rels = store.get_relations(&PersonId("p1".into())).await.unwrap();
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].relation.as_str(), "parent");

    store
        .remove_relation(
            &PersonId("p2".into()),
            &PersonId("p1".into()),
            &Relation::Parent,
        )
        .await
        .unwrap();
    let rels = store.get_relations(&PersonId("p1".into())).await.unwrap();
    assert_eq!(rels.len(), 0);
}

#[tokio::test]
async fn groups() {
    let store = test_store();
    store.add_person(&sample_person("p1", "Sam")).await.unwrap();
    store.add_person(&sample_person("p2", "Mom")).await.unwrap();

    store
        .add_group(&Group {
            id: GroupId("g1".into()),
            name: "Family Chat".into(),
            gateway_id: "discord".into(),
            external_id: "discord-family".into(),
            context: GroupContext::Family,
            members: vec![PersonId("p1".into()), PersonId("p2".into())],
        })
        .await
        .unwrap();

    let group = store
        .get_group(&GroupId("g1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group.name, "Family Chat");
    assert_eq!(group.members.len(), 2);

    store
        .add_person(&sample_person("p3", "Sister"))
        .await
        .unwrap();
    store
        .add_group_member(&GroupId("g1".into()), &PersonId("p3".into()))
        .await
        .unwrap();

    let group = store
        .get_group(&GroupId("g1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group.members.len(), 3);

    store
        .remove_group_member(&GroupId("g1".into()), &PersonId("p3".into()))
        .await
        .unwrap();
    let group = store
        .get_group(&GroupId("g1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group.members.len(), 2);
}

#[tokio::test]
async fn memory_subjects_association() {
    let store = test_store();
    let mem = Memory {
        id: MemoryId("m1".into()),
        kind: MemoryKind::Episodic,
        content: "Alice told me Bob got a new job".into(),
        source: MemorySource::Conversation {
            conversation_id: ConversationId("c1".into()),
            identity_id: None,
            profile_id: Some(ProfileId("profile-alice".into())),
            person_id: Some(PersonId("alice".into())),
            message_id: None,
        },
        importance: 0.7,
        sensitivity: 0.5,
        emotional_valence: 0.3,
        created_at: 1000,
        accessed_at: 1000,
        access_count: 0,
        tags: vec![],
        subjects: vec![
            MemorySubject::profile(
                ProfileId("profile-alice".into()),
                Some("speaker".into()),
                1.0,
            ),
            MemorySubject::person(PersonId("bob".into()), Some("mentioned".into()), 0.8),
        ],
        embedding: None,
    };
    store.store_memory(&mem).await.unwrap();

    let loaded = store
        .get_memory(&MemoryId("m1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.subjects.len(), 2);

    let results = store
        .recall(&RecallQuery::by_text("Bob", 10).with_person(PersonId("bob".into())))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    let results = store
        .recall(&RecallQuery::by_text("Bob", 10).with_person(PersonId("charlie".into())))
        .await
        .unwrap();
    assert_eq!(results.len(), 0);
}

#[tokio::test]
async fn memory_subjects_can_be_rewritten_without_legacy_person_links() {
    let store = test_store();
    let mut mem = sample_memory(
        "promotable",
        "Sam prefers concise updates",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    mem.subjects = vec![MemorySubject::profile(
        ProfileId("profile-sam".into()),
        Some("about".into()),
        1.0,
    )];
    store.store_memory(&mem).await.unwrap();

    store
        .update_memory(
            &mem.id,
            &MemoryUpdate {
                content: None,
                importance: None,
                sensitivity: None,
                emotional_valence: None,
                tags: None,
                subjects: Some(vec![MemorySubject::person(
                    PersonId("sam".into()),
                    Some("about".into()),
                    1.0,
                )]),
                embedding: None,
            },
        )
        .await
        .unwrap();

    let by_profile = store
        .recall(&RecallQuery::by_text("concise", 10).with_profile(ProfileId("profile-sam".into())))
        .await
        .unwrap();
    assert_eq!(by_profile.len(), 0);

    let by_person = store
        .recall(&RecallQuery::by_text("concise", 10).with_person(PersonId("sam".into())))
        .await
        .unwrap();
    assert_eq!(by_person.len(), 1);
}

#[tokio::test]
async fn fresh_schema_has_no_legacy_people_tables() {
    let store = test_store();
    let conn = store.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap();
    let tables = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect::<HashSet<_>>();

    assert!(tables.contains("persons"));
    assert!(tables.contains("memory_subjects"));
    assert!(!tables.contains("people"));
    assert!(!tables.contains("memory_people"));

    let thought_columns = conn
        .prepare("PRAGMA table_info(thoughts)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect::<HashSet<_>>();
    assert!(thought_columns.contains("subjects"));
    assert!(!thought_columns.contains("people"));
}

#[tokio::test]
async fn same_display_name_does_not_share_profile_memories() {
    let store = test_store();
    store.add_person(&sample_person("p1", "Sam")).await.unwrap();
    store.add_person(&sample_person("p2", "Sam")).await.unwrap();

    let identity_a = sample_identity("i1", "discord", "sam-a", "Sam");
    let profile_a = sample_profile("profile-a", "Sam");
    store.add_identity(&identity_a).await.unwrap();
    store.add_profile(&profile_a).await.unwrap();
    store
        .link_identity_to_profile(&identity_a.id, &profile_a.id, 1.0, None)
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &profile_a.id,
            &PersonId("p1".into()),
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();

    let identity_b = sample_identity("i2", "discord", "sam-b", "Sam");
    let profile_b = sample_profile("profile-b", "Sam");
    store.add_identity(&identity_b).await.unwrap();
    store.add_profile(&profile_b).await.unwrap();
    store
        .link_identity_to_profile(&identity_b.id, &profile_b.id, 1.0, None)
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &profile_b.id,
            &PersonId("p2".into()),
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();

    store
        .store_memory(&Memory {
            id: MemoryId("sam-a-city".into()),
            kind: MemoryKind::Semantic,
            content: "Sam said they are from Edmonton".into(),
            source: MemorySource::Conversation {
                conversation_id: ConversationId("c1".into()),
                identity_id: Some(identity_a.id.clone()),
                profile_id: Some(profile_a.id.clone()),
                person_id: Some(PersonId("p1".into())),
                message_id: None,
            },
            importance: 0.8,
            sensitivity: 0.0,
            emotional_valence: 0.0,
            created_at: 1000,
            accessed_at: 1000,
            access_count: 0,
            tags: vec![],
            subjects: vec![MemorySubject::profile(
                profile_a.id.clone(),
                Some("about".into()),
                1.0,
            )],
            embedding: None,
        })
        .await
        .unwrap();

    let current_profile_results = store
        .recall(&RecallQuery::by_text("Edmonton", 10).with_profile(profile_a.id))
        .await
        .unwrap();
    assert_eq!(current_profile_results.len(), 1);

    let same_name_other_profile_results = store
        .recall(&RecallQuery::by_text("Edmonton", 10).with_profile(profile_b.id))
        .await
        .unwrap();
    assert_eq!(same_name_other_profile_results.len(), 0);
}

#[tokio::test]
async fn profile_comm_style_is_stored_on_profile_not_person() {
    let store = test_store();
    store.add_person(&sample_person("p1", "Sam")).await.unwrap();
    let profile = sample_profile("profile-sam", "Sam");
    store.add_profile(&profile).await.unwrap();
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

    store
        .update_profile_comm_style(&profile.id, "Prefers concise replies")
        .await
        .unwrap();

    let loaded_profile = store.get_profile(&profile.id).await.unwrap().unwrap();
    assert_eq!(
        loaded_profile.comm_style.as_deref(),
        Some("Prefers concise replies")
    );

    let loaded_person = store
        .get_person(&PersonId("p1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded_person.comm_style, None);
}

#[tokio::test]
async fn detach_profile_removes_person_context_without_rewriting_memories() {
    let store = test_store();
    store
        .add_person(&sample_person("p1", "Alice"))
        .await
        .unwrap();

    let identity = sample_identity("i1", "telegram", "alice-alt", "Alice");
    let profile = sample_profile("profile-alice-alt", "Alice");
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

    store
        .store_memory(&Memory {
            id: MemoryId("profile-memory".into()),
            kind: MemoryKind::Semantic,
            content: "Alice alt prefers short messages".into(),
            source: MemorySource::Conversation {
                conversation_id: ConversationId("c1".into()),
                identity_id: Some(identity.id.clone()),
                profile_id: Some(profile.id.clone()),
                person_id: Some(PersonId("p1".into())),
                message_id: None,
            },
            importance: 0.8,
            sensitivity: 0.0,
            emotional_valence: 0.0,
            created_at: 1000,
            accessed_at: 1000,
            access_count: 0,
            tags: vec![],
            subjects: vec![MemorySubject::profile(
                profile.id.clone(),
                Some("about".into()),
                1.0,
            )],
            embedding: None,
        })
        .await
        .unwrap();

    assert!(
        store
            .get_person_for_profile(&profile.id)
            .await
            .unwrap()
            .is_some()
    );
    store
        .detach_profile_from_person(&profile.id, &PersonId("p1".into()), None)
        .await
        .unwrap();
    assert!(
        store
            .get_person_for_profile(&profile.id)
            .await
            .unwrap()
            .is_none()
    );

    let loaded = store
        .get_memory(&MemoryId("profile-memory".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.subjects[0].subject_id, profile.id.0);
}

#[tokio::test]
async fn behavior_directives() {
    let store = test_store();
    let sam = PersonId("sam".into());
    let mom = PersonId("mom".into());

    store
        .add_directive(&BehaviorDirective {
            id: "d1".into(),
            scope: DirectiveScope::Global,
            directive: "Never share private info between persons".into(),
            set_by: sam.clone(),
            priority: 0,
            active: true,
            created_at: 1000,
            expires_at: None,
        })
        .await
        .unwrap();

    store
        .add_directive(&BehaviorDirective {
            id: "d2".into(),
            scope: DirectiveScope::Person(mom.clone()),
            directive: "Be polite, no crude humor".into(),
            set_by: sam.clone(),
            priority: 10,
            active: true,
            created_at: 1000,
            expires_at: None,
        })
        .await
        .unwrap();

    store
        .add_directive(&BehaviorDirective {
            id: "d3".into(),
            scope: DirectiveScope::Authority(Authority::Default),
            directive: "Be warm and respectful".into(),
            set_by: sam.clone(),
            priority: 5,
            active: true,
            created_at: 1000,
            expires_at: None,
        })
        .await
        .unwrap();

    let directives = store
        .get_directives_for_context(&mom, &Authority::Default, None)
        .await
        .unwrap();
    assert_eq!(directives.len(), 3);
    assert_eq!(directives[0].id, "d2");
    assert_eq!(directives[1].id, "d3");
    assert_eq!(directives[2].id, "d1");

    store
        .update_directive("d2", None, Some(false), None, None)
        .await
        .unwrap();
    let directives = store
        .get_directives_for_context(&mom, &Authority::Default, None)
        .await
        .unwrap();
    assert_eq!(directives.len(), 2);

    assert!(store.remove_directive("d1").await.unwrap());
    assert!(!store.remove_directive("nonexistent").await.unwrap());

    let all = store.list_directives().await.unwrap();
    assert_eq!(all.len(), 2);
}
