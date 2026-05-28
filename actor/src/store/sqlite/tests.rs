use super::*;
use crate::identity::ClaimEvidence;
use crate::state::{ActorState, CoreTraits, DirectiveScope, GrowthConfig};
use crate::store::{
    ActionPromptSnapshotRecord, EventInboxRecord, IdentityDisclosureAudit, MemoryKind,
    MemorySource, MemoryType, MessageRole, PrivacyCategory, ReviewOutputAudit, TruthStatus,
    VisibilityScope,
};
use rusqlite::params;

mod helpers;
mod schema_tests;
mod thought_tests;
use helpers::*;

#[test]
fn slow_query_threshold_is_inclusive() {
    assert!(!sqlite_query_is_slow(
        std::time::Duration::from_millis(99),
        std::time::Duration::from_millis(100),
    ));
    assert!(sqlite_query_is_slow(
        std::time::Duration::from_millis(100),
        std::time::Duration::from_millis(100),
    ));
}

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
async fn memory_dedupe_key_upserts_existing_memory_subjects_and_index() {
    let store = test_store();
    let mut original = sample_memory(
        "memory-original",
        "Sam prefers short status updates",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    original.dedupe_key = Some("preference:profile-sam:status-length".into());
    original.subjects = vec![MemorySubject::profile(
        ProfileId("profile-sam".into()),
        Some("about".into()),
        1.0,
    )];
    let original_id = store.store_memory(&original).await.unwrap();

    let mut replacement = sample_memory(
        "memory-replacement",
        "Sam prefers concise deployment status updates",
        vec![0.2, 0.3, 0.4, 0.5],
    );
    replacement.dedupe_key = Some("preference:profile-sam:status-length".into());
    replacement.importance = 0.95;
    replacement.subjects = vec![MemorySubject::profile(
        ProfileId("profile-sam-work".into()),
        Some("about".into()),
        1.0,
    )];
    let replacement_id = store.store_memory(&replacement).await.unwrap();

    assert_eq!(replacement_id, original_id);
    assert!(
        store
            .get_memory(&MemoryId("memory-replacement".into()))
            .await
            .unwrap()
            .is_none()
    );

    let loaded = store.get_memory(&original_id).await.unwrap().unwrap();
    assert_eq!(
        loaded.content,
        "Sam prefers concise deployment status updates"
    );
    assert_eq!(loaded.importance, 0.95);
    assert_eq!(loaded.subjects.len(), 1);
    assert_eq!(loaded.subjects[0].subject_id, "profile-sam-work");

    let old_profile_results = store
        .recall(
            &RecallQuery::by_text("deployment", 10).with_profile(ProfileId("profile-sam".into())),
        )
        .await
        .unwrap();
    assert!(old_profile_results.is_empty());

    let new_profile_results = store
        .recall(
            &RecallQuery::by_text("deployment", 10)
                .with_profile(ProfileId("profile-sam-work".into())),
        )
        .await
        .unwrap();
    assert_eq!(new_profile_results.len(), 1);
    assert_eq!(new_profile_results[0].id, original_id);

    let conn = store.lock().unwrap();
    let mutation_count: u32 = conn
        .query_row(
            "SELECT count(*) FROM memory_mutations WHERE memory_id = ?1",
            params![original_id.0],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mutation_count, 2);
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
async fn text_recall_prefers_search_relevance_over_recency() {
    let store = test_store();
    let mut relevant = sample_memory(
        "older-relevant",
        "Sam asked for a kubernetes budget review",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    relevant.created_at = 1000;
    relevant.importance = 0.5;
    let mut recent = sample_memory(
        "newer-weaker",
        "Sam mentioned kubernetes",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    recent.created_at = 2000;
    recent.importance = 0.5;

    store.store_memory(&recent).await.unwrap();
    store.store_memory(&relevant).await.unwrap();

    let results = store
        .recall(&RecallQuery::by_text("kubernetes budget", 2))
        .await
        .unwrap();

    assert_eq!(results[0].id.0, "older-relevant");
}

#[tokio::test]
async fn fallback_text_recall_prefers_prefix_match_over_substring_importance() {
    let store = test_store();
    let mut prefix_match = sample_memory(
        "prefix-match",
        "Sam keeps a deployment checklist",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    prefix_match.created_at = 1000;
    prefix_match.importance = 0.6;
    let mut substring_match = sample_memory(
        "substring-match",
        "Sam archived a redeployment note",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    substring_match.created_at = 2000;
    substring_match.importance = 1.0;

    store.store_memory(&substring_match).await.unwrap();
    store.store_memory(&prefix_match).await.unwrap();

    let results = store
        .recall(&RecallQuery::by_text("deploym", 2))
        .await
        .unwrap();

    assert_eq!(results[0].id.0, "prefix-match");
    assert_eq!(results[1].id.0, "substring-match");
}

#[tokio::test]
async fn scoped_text_recall_filters_before_limiting() {
    let store = test_store();
    for idx in 0..6 {
        let mut memory = sample_memory(
            &format!("global-{idx}"),
            "deployment schedule changed",
            vec![0.1, 0.2, 0.3, 0.4],
        );
        memory.created_at = 10_000 + idx;
        memory.subjects = vec![MemorySubject::profile(
            ProfileId("profile-other".into()),
            Some("about".into()),
            1.0,
        )];
        store.store_memory(&memory).await.unwrap();
    }

    let mut scoped = sample_memory(
        "scoped",
        "deployment schedule changed",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    scoped.created_at = 1000;
    scoped.subjects = vec![MemorySubject::profile(
        ProfileId("profile-target".into()),
        Some("about".into()),
        1.0,
    )];
    store.store_memory(&scoped).await.unwrap();

    let results = store
        .recall(
            &RecallQuery::by_text("deployment", 1).with_profile(ProfileId("profile-target".into())),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "scoped");
}

#[tokio::test]
async fn memory_type_recall_filters_before_limiting() {
    let store = test_store();
    let profile = ProfileId("profile-target".into());
    for idx in 0..60 {
        let mut memory = sample_memory(
            &format!("recent-fact-{idx}"),
            "recent generic observation",
            vec![0.1, 0.2, 0.3, 0.4],
        );
        memory.created_at = 20_000 + idx;
        memory.memory_type = MemoryType::Fact;
        memory.subjects = vec![MemorySubject::profile(
            profile.clone(),
            Some("about".into()),
            1.0,
        )];
        store.store_memory(&memory).await.unwrap();
    }

    let mut boundary = sample_memory(
        "older-boundary",
        "Do not send deployment reminders after 8pm.",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    boundary.created_at = 1000;
    boundary.memory_type = MemoryType::Boundary;
    boundary.subjects = vec![MemorySubject::profile(
        profile.clone(),
        Some("about".into()),
        1.0,
    )];
    store.store_memory(&boundary).await.unwrap();

    let results = store
        .recall(
            &RecallQuery::by_text("", 1)
                .with_profile(profile)
                .with_memory_type(MemoryType::Boundary),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "older-boundary");
}

#[tokio::test]
async fn default_recall_excludes_sensitive_memories_until_opted_in() {
    let store = test_store();
    let mut public = sample_memory(
        "public",
        "Sam likes public deployment summaries",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    public.sensitivity = 0.2;
    public.privacy_category = PrivacyCategory::Personal;
    let mut secret = sample_memory(
        "secret",
        "Sam's secret deployment credential rotation detail",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    secret.sensitivity = 0.95;
    secret.privacy_category = PrivacyCategory::Secret;
    let mut sensitive = sample_memory(
        "sensitive",
        "Sam has a sensitive deployment-related medical appointment",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    sensitive.sensitivity = 0.1;
    sensitive.privacy_category = PrivacyCategory::Sensitive;

    store.store_memory(&public).await.unwrap();
    store.store_memory(&secret).await.unwrap();
    store.store_memory(&sensitive).await.unwrap();

    let default_results = store
        .recall(&RecallQuery::by_text("deployment", 10))
        .await
        .unwrap();
    assert_eq!(default_results.len(), 1);
    assert_eq!(default_results[0].id.0, "public");

    let sensitive_results = store
        .recall(&RecallQuery::by_text("deployment", 10).include_sensitive())
        .await
        .unwrap();
    let ids = sensitive_results
        .iter()
        .map(|memory| memory.id.0.as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"public"));
    assert!(ids.contains(&"sensitive"));
    assert!(ids.contains(&"secret"));
}

#[tokio::test]
async fn default_recall_excludes_superseded_and_outdated_memories_until_opted_in() {
    let store = test_store();
    let active = sample_memory(
        "active",
        "Sam wants deployment summaries to stay concise",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    let mut superseded = sample_memory(
        "superseded",
        "Sam previously wanted deployment summaries to include every detail",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    superseded.superseded_by = Some(MemoryId("active".into()));
    let mut outdated = sample_memory(
        "outdated",
        "Sam used to prefer deployment summaries after midnight",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    outdated.truth_status = TruthStatus::Outdated;

    store.store_memory(&active).await.unwrap();
    store.store_memory(&superseded).await.unwrap();
    store.store_memory(&outdated).await.unwrap();

    let default_results = store
        .recall(&RecallQuery::by_text("deployment summaries", 10))
        .await
        .unwrap();
    assert_eq!(default_results.len(), 1);
    assert_eq!(default_results[0].id.0, "active");

    let historical_results = store
        .recall(&RecallQuery::by_text("deployment summaries", 10).include_superseded())
        .await
        .unwrap();
    let ids = historical_results
        .iter()
        .map(|memory| memory.id.0.as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"active"));
    assert!(ids.contains(&"superseded"));
    assert!(ids.contains(&"outdated"));
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
async fn scoped_embedding_recall_filters_before_vector_limit() {
    let store = test_store();
    let target_profile = ProfileId("profile-target".into());

    for idx in 0..220 {
        let mut memory = sample_memory(
            &format!("other-near-{idx}"),
            "unrelated deployment memory",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        memory.created_at = 10_000 + idx;
        memory.subjects = vec![MemorySubject::profile(
            ProfileId("profile-other".into()),
            Some("about".into()),
            1.0,
        )];
        store.store_memory(&memory).await.unwrap();
    }

    let mut scoped = sample_memory(
        "scoped-far",
        "target profile deployment memory",
        vec![0.0, 1.0, 0.0, 0.0],
    );
    scoped.subjects = vec![MemorySubject::profile(
        target_profile.clone(),
        Some("about".into()),
        1.0,
    )];
    store.store_memory(&scoped).await.unwrap();

    let results = store
        .recall(
            &RecallQuery::by_embedding(vec![1.0, 0.0, 0.0, 0.0], 1).with_profile(target_profile),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "scoped-far");
}

#[tokio::test]
async fn scoped_embedding_recall_filters_sensitivity_after_vector_ranking() {
    let store = test_store();
    let target_profile = ProfileId("profile-target".into());

    for idx in 0..25 {
        let mut memory = sample_memory(
            &format!("sensitive-near-{idx}"),
            "sensitive deployment memory",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        memory.privacy_category = PrivacyCategory::Sensitive;
        memory.sensitivity = 0.8;
        memory.subjects = vec![MemorySubject::profile(
            target_profile.clone(),
            Some("about".into()),
            1.0,
        )];
        store.store_memory(&memory).await.unwrap();
    }

    let mut public = sample_memory(
        "public-far",
        "public target profile deployment memory",
        vec![0.0, 1.0, 0.0, 0.0],
    );
    public.subjects = vec![MemorySubject::profile(
        target_profile.clone(),
        Some("about".into()),
        1.0,
    )];
    store.store_memory(&public).await.unwrap();

    let results = store
        .recall(
            &RecallQuery::by_embedding(vec![1.0, 0.0, 0.0, 0.0], 1).with_profile(target_profile),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "public-far");
}

#[tokio::test]
async fn embedding_recall_filters_memory_type_before_vector_limit() {
    let store = test_store();

    for idx in 0..220 {
        let mut memory = sample_memory(
            &format!("near-fact-{idx}"),
            "generic deployment fact",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        memory.created_at = 10_000 + idx;
        memory.memory_type = MemoryType::Fact;
        store.store_memory(&memory).await.unwrap();
    }

    let mut boundary = sample_memory(
        "far-boundary",
        "Do not send deployment reminders after 8pm.",
        vec![0.0, 1.0, 0.0, 0.0],
    );
    boundary.memory_type = MemoryType::Boundary;
    store.store_memory(&boundary).await.unwrap();

    let results = store
        .recall(
            &RecallQuery::by_embedding(vec![1.0, 0.0, 0.0, 0.0], 1)
                .with_memory_type(MemoryType::Boundary),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "far-boundary");
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
    let conn = store.lock().unwrap();
    let operation: String = conn
        .query_row(
            "SELECT operation FROM memory_mutations WHERE memory_id = ?1 ORDER BY id DESC LIMIT 1",
            params!["m1"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(operation, "forget");
    drop(conn);

    assert!(!store.forget(&MemoryId("m1".into())).await.unwrap());

    store
        .store_memory(&sample_memory(
            "m2",
            "owner deleted",
            vec![0.4, 0.3, 0.2, 0.1],
        ))
        .await
        .unwrap();
    assert!(
        store
            .forget_with_reason(&MemoryId("m2".into()), Some("owner requested deletion"))
            .await
            .unwrap()
    );
    let conn = store.lock().unwrap();
    let reason: Option<String> = conn
        .query_row(
            "SELECT reason FROM memory_mutations WHERE memory_id = ?1 ORDER BY id DESC LIMIT 1",
            params!["m2"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reason.as_deref(), Some("owner requested deletion"));
    drop(conn);

    let mutations = store
        .memory_mutations_for_memory(&MemoryId("m2".into()), 10)
        .await
        .unwrap();
    assert_eq!(mutations[0].operation, "forget");
    assert_eq!(
        mutations[0].reason.as_deref(),
        Some("owner requested deletion")
    );
    assert!(
        mutations
            .iter()
            .any(|mutation| mutation.operation == "create")
    );
}

#[tokio::test]
async fn prune_stale_memories_removes_only_low_signal_expired_or_superseded_rows() {
    let store = test_store();
    let mut expired_low_signal =
        sample_memory("expired-low", "expired noise", vec![1.0, 0.0, 0.0, 0.0]);
    expired_low_signal.importance = 0.1;
    expired_low_signal.confidence = 0.2;
    expired_low_signal.sensitivity = 0.1;
    expired_low_signal.expires_at = Some(900);
    expired_low_signal.accessed_at = 800;

    let mut superseded_low_signal =
        sample_memory("superseded-low", "old duplicate", vec![0.0, 1.0, 0.0, 0.0]);
    superseded_low_signal.importance = 0.1;
    superseded_low_signal.confidence = 0.2;
    superseded_low_signal.sensitivity = 0.1;
    superseded_low_signal.superseded_by = Some(MemoryId("replacement".into()));
    superseded_low_signal.accessed_at = 100;

    let mut active_low_signal =
        sample_memory("active-low", "active but low", vec![0.0, 0.0, 1.0, 0.0]);
    active_low_signal.importance = 0.1;
    active_low_signal.confidence = 0.2;
    active_low_signal.sensitivity = 0.1;
    active_low_signal.accessed_at = 100;

    let mut important_expired = sample_memory(
        "important-expired",
        "important expired",
        vec![0.0, 0.0, 0.0, 1.0],
    );
    important_expired.importance = 0.9;
    important_expired.confidence = 0.2;
    important_expired.sensitivity = 0.1;
    important_expired.expires_at = Some(900);

    let mut protected_boundary = sample_memory(
        "protected-boundary",
        "do not contact after 9pm",
        vec![0.5, 0.0, 0.0, 0.0],
    );
    protected_boundary.memory_type = MemoryType::Boundary;
    protected_boundary.importance = 0.1;
    protected_boundary.confidence = 0.2;
    protected_boundary.sensitivity = 0.1;
    protected_boundary.expires_at = Some(900);

    let mut secret_expired =
        sample_memory("secret-expired", "secret expired", vec![0.0, 0.5, 0.0, 0.0]);
    secret_expired.importance = 0.1;
    secret_expired.confidence = 0.2;
    secret_expired.sensitivity = 0.1;
    secret_expired.privacy_category = PrivacyCategory::Secret;
    secret_expired.expires_at = Some(900);

    let mut sensitive_expired = sample_memory(
        "sensitive-expired",
        "sensitive expired health reminder",
        vec![0.0, 0.0, 0.5, 0.0],
    );
    sensitive_expired.importance = 0.1;
    sensitive_expired.confidence = 0.2;
    sensitive_expired.sensitivity = 0.1;
    sensitive_expired.privacy_category = PrivacyCategory::Sensitive;
    sensitive_expired.expires_at = Some(900);

    for memory in [
        expired_low_signal,
        superseded_low_signal,
        active_low_signal,
        important_expired,
        protected_boundary,
        secret_expired,
        sensitive_expired,
    ] {
        store.store_memory(&memory).await.unwrap();
    }

    let pruned = store
        .prune_stale_memories(1000, 500, 0.3, 0.5, 0.4, 100)
        .await
        .unwrap();
    assert_eq!(pruned, 2);
    assert!(
        store
            .get_memory(&MemoryId("expired-low".into()))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .get_memory(&MemoryId("superseded-low".into()))
            .await
            .unwrap()
            .is_none()
    );
    for id in [
        "active-low",
        "important-expired",
        "protected-boundary",
        "secret-expired",
        "sensitive-expired",
    ] {
        assert!(
            store
                .get_memory(&MemoryId(id.into()))
                .await
                .unwrap()
                .is_some(),
            "{id} should be retained"
        );
    }

    let conn = store.lock().unwrap();
    let operation: String = conn
        .query_row(
            "SELECT operation FROM memory_mutations WHERE memory_id = ?1 ORDER BY id DESC LIMIT 1",
            params!["expired-low"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(operation, "prune");
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
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-1".into()),
                sender_external_id: Some("local".into()),
                reply_external_id: Some("local".into()),
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
                source_gateway_id: None,
                source_message_id: None,
                sender_external_id: None,
                reply_external_id: Some("local".into()),
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
    assert_eq!(convs[0].summary_version, 0);

    store
        .update_conversation_summary(
            &conv,
            "Sam said hello and the actor replied.",
            &[String::from("msg-1")],
        )
        .await
        .unwrap();

    let convs = store.list_conversations().await.unwrap();
    assert_eq!(
        convs[0].summary.as_deref(),
        Some("Sam said hello and the actor replied.")
    );
    assert_eq!(
        convs[0].summary_covered_message_ids,
        vec![String::from("msg-1")]
    );
    assert!(convs[0].summary_updated_at.is_some());
    assert_eq!(convs[0].summary_version, 1);

    store
        .update_conversation_summary(
            &conv,
            "Sam said hello; the actor replied and the next message is covered.",
            &[String::from("msg-2"), String::from("msg-1")],
        )
        .await
        .unwrap();

    let convs = store.list_conversations().await.unwrap();
    assert_eq!(
        convs[0].summary_covered_message_ids,
        vec![String::from("msg-1"), String::from("msg-2")]
    );
    assert_eq!(convs[0].summary_version, 2);
}

#[tokio::test]
async fn inbound_message_append_is_idempotent_and_preserves_group_context() {
    let store = test_store();
    let conv = ConversationId("discord:channel-1".into());
    let group = GroupId("discord:guild-1".into());
    let msg = StoredMessage {
        timestamp: 1000,
        role: MessageRole::User,
        content: "hello group".into(),
        identity: Some(IdentityId("identity-a".into())),
        profile: Some(ProfileId("profile-a".into())),
        person: Some(PersonId("person-a".into())),
        source_gateway_id: Some("discord".into()),
        source_message_id: Some("discord-msg-1".into()),
        sender_external_id: Some("author-a".into()),
        reply_external_id: Some("channel-1".into()),
        metadata: serde_json::json!({"message_id": "discord-msg-1"}),
    };

    store
        .append_message(&conv, Some("discord"), Some(&group), &msg)
        .await
        .unwrap();
    store
        .append_message(&conv, Some("discord"), Some(&group), &msg)
        .await
        .unwrap();

    let msgs = store.get_messages(&conv, 10, None).await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].source_message_id.as_deref(), Some("discord-msg-1"));
    assert_eq!(msgs[0].sender_external_id.as_deref(), Some("author-a"));
    assert_eq!(msgs[0].reply_external_id.as_deref(), Some("channel-1"));

    let convs = store.list_conversations().await.unwrap();
    assert_eq!(convs[0].message_count, 1);
    assert_eq!(convs[0].gateway_id.as_deref(), Some("discord"));
    assert_eq!(convs[0].group.as_ref(), Some(&group));
}

#[tokio::test]
async fn message_edit_and_delete_update_visible_history_and_action_sources() {
    let store = test_store();
    let conv = ConversationId("discord:channel-1".into());

    store
        .append_message(
            &conv,
            Some("discord"),
            None,
            &StoredMessage {
                timestamp: 1000,
                role: MessageRole::User,
                content: "before edit".into(),
                identity: None,
                profile: Some(ProfileId("profile-a".into())),
                person: Some(PersonId("person-a".into())),
                source_gateway_id: Some("discord".into()),
                source_message_id: Some("discord-msg-1".into()),
                sender_external_id: Some("author-a".into()),
                reply_external_id: Some("channel-1".into()),
                metadata: serde_json::json!({"message_id": "discord-msg-1"}),
            },
        )
        .await
        .unwrap();
    store
        .append_action_message(&ActionMessageRecord {
            action_id: "action-1".into(),
            role: "user".into(),
            conversation: Some(conv.clone()),
            source_gateway_id: Some("discord".into()),
            source_message_id: Some("discord-msg-1".into()),
            sender_external_id: Some("author-a".into()),
            reply_external_id: Some("channel-1".into()),
            content: Some("before edit".into()),
            created_at: 1000,
        })
        .await
        .unwrap();

    assert!(
        store
            .update_message_content_by_source(
                &conv,
                "discord",
                "discord-msg-1",
                "after edit",
                1100,
            )
            .await
            .unwrap()
    );
    let messages = store.get_messages(&conv, 10, None).await.unwrap();
    assert_eq!(messages[0].content, "after edit");
    assert_eq!(messages[0].metadata["edited"], true);
    assert_eq!(messages[0].metadata["edited_at"], 1100);
    let transcript = store.action_transcript("action-1").await.unwrap();
    assert_eq!(
        transcript.messages[0].content.as_deref(),
        Some("after edit")
    );

    assert!(
        store
            .mark_message_deleted_by_source(&conv, "discord", "discord-msg-1", 1200)
            .await
            .unwrap()
    );
    let messages = store.get_messages(&conv, 10, None).await.unwrap();
    assert_eq!(messages[0].content, "[message deleted]");
    assert_eq!(messages[0].metadata["deleted"], true);
    assert_eq!(messages[0].metadata["deleted_at"], 1200);
    let transcript = store.action_transcript("action-1").await.unwrap();
    assert_eq!(
        transcript.messages[0].content.as_deref(),
        Some("[message deleted]")
    );
}

#[tokio::test]
async fn action_transcript_records_run_turn_tools_messages_and_review_watermark() {
    let store = test_store();
    let action_id = "action-1";

    store
        .start_action_run(&ActionRunRecord {
            action_id: action_id.into(),
            kind: "respond".into(),
            task: "Respond to message".into(),
            conversation: Some(ConversationId("relay:local".into())),
            started_at: 1000,
            ended_at: None,
            status: "running".into(),
            responded: false,
            attempts: 0,
        })
        .await
        .unwrap();
    store
        .append_action_message(&ActionMessageRecord {
            action_id: action_id.into(),
            role: "user".into(),
            conversation: Some(ConversationId("relay:local".into())),
            source_gateway_id: Some("relay".into()),
            source_message_id: Some("msg-1".into()),
            sender_external_id: Some("local".into()),
            reply_external_id: Some("local".into()),
            content: Some("hello".into()),
            created_at: 1001,
        })
        .await
        .unwrap();
    store
        .append_action_message(&ActionMessageRecord {
            action_id: action_id.into(),
            role: "user".into(),
            conversation: Some(ConversationId("relay:local".into())),
            source_gateway_id: Some("relay".into()),
            source_message_id: Some("msg-1".into()),
            sender_external_id: Some("local".into()),
            reply_external_id: Some("local".into()),
            content: Some("duplicate delivery".into()),
            created_at: 1002,
        })
        .await
        .unwrap();
    store
        .append_action_turn(&ActionTurnRecord {
            action_id: action_id.into(),
            turn: 0,
            attempt: 1,
            prompt_hash: "abc123".into(),
            model: Some("model-a".into()),
            finish: Some("tool_calls".into()),
            input_tokens: Some(10),
            output_tokens: Some(3),
            text_len: 4,
            reasoning_len: 0,
            tool_call_count: 1,
            created_at: 1002,
        })
        .await
        .unwrap();
    store
        .record_prompt_snapshot(&ActionPromptSnapshotRecord {
            action_id: action_id.into(),
            turn: 0,
            attempt: 1,
            prompt_hash: "abc123".into(),
            messages: serde_json::json!([
                {
                    "role": "system",
                    "content": "System prompt with current profile context."
                },
                {
                    "role": "user",
                    "content": "hello"
                }
            ]),
            created_at: 1002,
        })
        .await
        .unwrap();
    store
        .append_tool_call(&ToolCallRecord {
            action_id: action_id.into(),
            turn: 0,
            call_id: "call-1".into(),
            name: "send_message".into(),
            args: serde_json::json!({"content": "hi"}),
            result: serde_json::json!({"result": "Message sent."}),
            success: true,
            started_at: 1003,
            ended_at: 1004,
        })
        .await
        .unwrap();
    store
        .finish_action_run(
            action_id,
            1005,
            "completed",
            true,
            1,
            vec![MemoryId("memory-formed".into())],
            vec![MemoryId("memory-recalled".into())],
        )
        .await
        .unwrap();

    assert!(
        store
            .mark_review_scheduled(action_id, "review-1", 1006)
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_review_scheduled(action_id, "review-2", 1007)
            .await
            .unwrap()
    );
    assert!(store.action_review_scheduled(action_id).await.unwrap());
    store
        .record_review_output(&ReviewOutputAudit {
            id: "review-output-1".into(),
            review_action_id: "review-1".into(),
            source_action_id: Some(action_id.into()),
            input: serde_json::json!({
                "memories": [{
                    "content": "Sam prefers concise summaries.",
                    "evidence_message_ids": ["msg-1"]
                }]
            }),
            result: serde_json::json!({
                "status": "applied",
                "memories": 1,
                "skipped": []
            }),
            applied_at: 1007,
        })
        .await
        .unwrap();

    let conn = store.lock().unwrap();
    let (status, responded, attempts): (String, i32, u32) = conn
        .query_row(
            "SELECT status, responded, attempts FROM action_runs WHERE action_id = ?1",
            params![action_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "completed");
    assert_eq!(responded, 1);
    assert_eq!(attempts, 1);

    let tool_count: u32 = conn
        .query_row(
            "SELECT count(*) FROM action_tool_calls WHERE action_id = ?1",
            params![action_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tool_count, 1);

    let message_count: u32 = conn
        .query_row(
            "SELECT count(*) FROM action_messages WHERE action_id = ?1",
            params![action_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(message_count, 1);
    drop(conn);

    let transcript = store.action_transcript(action_id).await.unwrap();
    let run = transcript.run.expect("action run");
    assert_eq!(run.status, "completed");
    assert!(run.responded);
    assert_eq!(run.attempts, 1);
    assert_eq!(
        transcript.memories_formed,
        vec![MemoryId("memory-formed".into())]
    );
    assert_eq!(
        transcript.recalled_memory_ids,
        vec![MemoryId("memory-recalled".into())]
    );
    assert_eq!(transcript.messages.len(), 1);
    assert_eq!(transcript.messages[0].content.as_deref(), Some("hello"));
    assert_eq!(transcript.turns.len(), 1);
    assert_eq!(transcript.turns[0].model.as_deref(), Some("model-a"));
    assert_eq!(transcript.prompt_snapshots.len(), 1);
    assert_eq!(transcript.prompt_snapshots[0].prompt_hash, "abc123");
    assert_eq!(
        transcript.prompt_snapshots[0].messages[1]["content"],
        "hello"
    );
    assert_eq!(transcript.tool_calls.len(), 1);
    assert_eq!(transcript.tool_calls[0].name, "send_message");
    assert_eq!(transcript.tool_calls[0].result["result"], "Message sent.");
    let review_outputs = store.review_outputs_for_action("review-1").await.unwrap();
    assert_eq!(review_outputs.len(), 1);
    assert_eq!(
        review_outputs[0].source_action_id.as_deref(),
        Some(action_id)
    );
    assert_eq!(
        review_outputs[0].input["memories"][0]["content"],
        "[redacted]"
    );
    assert_eq!(
        review_outputs[0].input["memories"][0]["evidence_message_ids"][0],
        "msg-1"
    );
    assert_eq!(review_outputs[0].result["memories"], 1);

    let source_review_outputs = store
        .review_outputs_for_source_action(action_id)
        .await
        .unwrap();
    assert_eq!(source_review_outputs.len(), 1);
    assert_eq!(source_review_outputs[0].review_action_id, "review-1");
}

#[tokio::test]
async fn tool_call_transcripts_redact_sensitive_args_and_results() {
    let store = test_store();
    store
        .append_tool_call(&ToolCallRecord {
            action_id: "action-redact".into(),
            turn: 0,
            call_id: "call-redact".into(),
            name: "get_person".into(),
            args: serde_json::json!({
                "include_identities": true,
                "delivery_required": true,
                "reason": "deliver to target",
                "external_id": "target-external-id",
            }),
            result: serde_json::json!({
                "result": serde_json::json!({
                    "identities": [{
                        "gateway_id": "discord",
                        "external_id": "target-external-id",
                        "display_name": "Target"
                    }],
                    "messages": [{
                        "content": "private deployment detail"
                    }]
                })
                .to_string()
            }),
            success: true,
            started_at: 1000,
            ended_at: 1001,
        })
        .await
        .unwrap();

    let conn = store.lock().unwrap();
    let (args_json, result_json): (String, String) = conn
        .query_row(
            "SELECT args_json, result_json FROM action_tool_calls WHERE action_id = ?1",
            params!["action-redact"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let args: serde_json::Value = serde_json::from_str(&args_json).unwrap();
    let result: serde_json::Value = serde_json::from_str(&result_json).unwrap();
    let inner_result: serde_json::Value =
        serde_json::from_str(result["result"].as_str().unwrap()).unwrap();

    assert_eq!(args["external_id"], "[redacted]");
    assert_eq!(args["reason"], "[redacted]");
    assert_eq!(inner_result["identities"][0]["external_id"], "[redacted]");
    assert_eq!(inner_result["messages"][0]["content"], "[redacted]");
    assert_eq!(inner_result["identities"][0]["gateway_id"], "discord");
}

#[tokio::test]
async fn debug_views_return_bounded_recent_records() {
    let store = test_store();
    store
        .add_profile(&sample_profile("profile-debug", "Debug User"))
        .await
        .unwrap();
    store
        .add_identity(&sample_identity(
            "identity-debug",
            "discord",
            "debug-user",
            "Debug User",
        ))
        .await
        .unwrap();
    store
        .add_person(&sample_person("person-debug", "Debug User"))
        .await
        .unwrap();
    store
        .link_identity_to_profile(
            &IdentityId("identity-debug".into()),
            &ProfileId("profile-debug".into()),
            0.9,
            Some(&serde_json::json!({"message_id": "msg-link-profile"})),
        )
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &ProfileId("profile-debug".into()),
            &PersonId("person-debug".into()),
            PersonProfileStatus::Verified,
            0.95,
            Some(&serde_json::json!({"message_id": "msg-link-person"})),
        )
        .await
        .unwrap();
    store
        .add_group(&Group {
            id: GroupId("group-debug".into()),
            name: "Debug Group".into(),
            gateway_id: "discord".into(),
            external_id: "debug-channel".into(),
            context: GroupContext::Work,
            members: vec![PersonId("person-debug".into())],
        })
        .await
        .unwrap();
    let mut memory = sample_memory(
        "debug-memory",
        "Debug memory content",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    memory.created_at = 2000;
    memory.subjects = vec![MemorySubject::profile(
        ProfileId("profile-debug".into()),
        Some("about".into()),
        1.0,
    )];
    store.store_memory(&memory).await.unwrap();

    store
        .create_intent(&IntentRecord {
            id: "intent-debug".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Inspect debug snapshot".into(),
            person: None,
            profile: Some(ProfileId("profile-debug".into())),
            conversation: None,
            fire_at: Some(3000),
            condition: None,
            recurrence: None,
            priority: 80,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: 1000,
            updated_at: 1000,
            last_fired_at: None,
            owner_approved: false,
        })
        .await
        .unwrap();
    store
        .create_intent(&IntentRecord {
            id: "intent-pending-approval-debug".into(),
            kind: "scheduled".into(),
            status: "pending_approval".into(),
            task: "Inspect pending approval debug snapshot".into(),
            person: None,
            profile: Some(ProfileId("profile-debug".into())),
            conversation: None,
            fire_at: Some(3001),
            condition: None,
            recurrence: None,
            priority: 70,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: 1001,
            updated_at: 1001,
            last_fired_at: None,
            owner_approved: false,
        })
        .await
        .unwrap();
    store
        .start_action_run(&ActionRunRecord {
            action_id: "action-debug".into(),
            kind: "respond".into(),
            task: "Debug action".into(),
            conversation: Some(ConversationId("relay:local".into())),
            started_at: 4000,
            ended_at: Some(4001),
            status: "completed".into(),
            responded: true,
            attempts: 1,
        })
        .await
        .unwrap();
    store
        .start_action_run(&ActionRunRecord {
            action_id: "review-action-debug".into(),
            kind: "review".into(),
            task: "Review debug action".into(),
            conversation: Some(ConversationId("relay:local".into())),
            started_at: 3999,
            ended_at: Some(4002),
            status: "completed".into(),
            responded: false,
            attempts: 1,
        })
        .await
        .unwrap();
    store
        .mark_review_scheduled("action-debug", "review-action-debug", 4001)
        .await
        .unwrap();
    store
        .record_review_output(&ReviewOutputAudit {
            id: "review-debug".into(),
            review_action_id: "review-action-debug".into(),
            source_action_id: Some("action-debug".into()),
            input: serde_json::json!({"conversation_summary": {"summary": "debug"}}),
            result: serde_json::json!({"conversation_summary": 1}),
            applied_at: 4002,
        })
        .await
        .unwrap();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-failed-old".into(),
            kind: "message".into(),
            payload: serde_json::json!({"content": "older private payload"}),
            status: "pending".into(),
            due_at: 3900,
            attempts: 0,
            dedupe_key: Some("event-failed-old".into()),
            created_at: 3900,
            updated_at: 3900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-failed-new".into(),
            kind: "message".into(),
            payload: serde_json::json!({"content": "newer private payload"}),
            status: "pending".into(),
            due_at: 3901,
            attempts: 0,
            dedupe_key: Some("event-failed-new".into()),
            created_at: 3901,
            updated_at: 3901,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();
    store
        .mark_event_failed("event-failed-old", 4003, Some("older malformed payload"))
        .await
        .unwrap();
    store
        .mark_event_failed("event-failed-new", 4004, Some("newer malformed payload"))
        .await
        .unwrap();

    let profiles = store.list_profiles().await.unwrap();
    assert_eq!(profiles[0].id.0, "profile-debug");

    let memories = store.debug_recent_memories(1).await.unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].id.0, "debug-memory");
    assert_eq!(memories[0].subjects[0].subject_id, "profile-debug");
    let memory_subjects = store.debug_memory_subjects(1).await.unwrap();
    assert_eq!(memory_subjects.len(), 1);
    assert_eq!(memory_subjects[0].subject_type, MemorySubjectType::Profile);
    assert_eq!(memory_subjects[0].subject_id, "profile-debug");
    assert_eq!(memory_subjects[0].memory_count, 1);
    assert_eq!(memory_subjects[0].latest_memory_ids[0].0, "debug-memory");
    let profile_links = store.debug_profile_identity_links(1).await.unwrap();
    assert_eq!(profile_links.len(), 1);
    assert_eq!(profile_links[0].profile_id.0, "profile-debug");
    assert_eq!(profile_links[0].identity_id.0, "identity-debug");
    assert_eq!(
        profile_links[0].evidence.as_ref().unwrap()["message_id"],
        "msg-link-profile"
    );
    let person_links = store.debug_person_profile_links(1).await.unwrap();
    assert_eq!(person_links.len(), 1);
    assert_eq!(person_links[0].person_id.0, "person-debug");
    assert_eq!(person_links[0].profile_id.0, "profile-debug");
    assert_eq!(
        person_links[0].evidence.as_ref().unwrap()["message_id"],
        "msg-link-person"
    );
    let groups = store.debug_groups(1).await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id.0, "group-debug");
    assert_eq!(groups[0].members, vec![PersonId("person-debug".into())]);

    let intents = store.debug_active_intents(1).await.unwrap();
    assert_eq!(intents[0].id, "intent-debug");
    let intents = store.debug_active_intents(10).await.unwrap();
    assert!(intents.iter().any(|intent| intent.id == "intent-debug"));
    assert!(
        intents
            .iter()
            .any(|intent| intent.id == "intent-pending-approval-debug"
                && intent.status == "pending_approval")
    );

    let actions = store.debug_recent_action_runs(1).await.unwrap();
    assert_eq!(actions[0].action_id, "action-debug");

    let reviews = store.debug_recent_review_outputs(1).await.unwrap();
    assert_eq!(reviews[0].id, "review-debug");
    assert_eq!(reviews[0].source_action_id.as_deref(), Some("action-debug"));

    let review_jobs = store.debug_recent_review_jobs(1).await.unwrap();
    assert_eq!(review_jobs.len(), 1);
    assert_eq!(review_jobs[0].source_action_id, "action-debug");
    assert_eq!(review_jobs[0].review_action_id, "review-action-debug");
    assert_eq!(review_jobs[0].source_kind.as_deref(), Some("respond"));
    assert_eq!(review_jobs[0].source_status.as_deref(), Some("completed"));
    assert_eq!(review_jobs[0].review_status.as_deref(), Some("completed"));
    assert_eq!(review_jobs[0].output_count, 1);
    assert_eq!(review_jobs[0].last_applied_at, Some(4002));

    let mutations = store.debug_recent_memory_mutations(1).await.unwrap();
    assert_eq!(mutations.len(), 1);
    assert_eq!(mutations[0].memory.0, "debug-memory");
    assert_eq!(mutations[0].operation, "create");
    assert_eq!(mutations[0].data["input_memory_id"], "debug-memory");

    let failed_events = store.debug_recent_failed_events(1).await.unwrap();
    assert_eq!(failed_events.len(), 1);
    assert_eq!(failed_events[0].id, "event-failed-new");
    assert_eq!(failed_events[0].status, "failed");
    assert_eq!(failed_events[0].attempts, 1);
    assert_eq!(
        failed_events[0].last_error.as_deref(),
        Some("newer malformed payload")
    );
    let failed_event_json = serde_json::to_value(&failed_events[0]).unwrap();
    assert!(failed_event_json.get("payload").is_none());
}

#[tokio::test]
async fn intents_are_persisted_updated_due_and_fired_once() {
    let store = test_store();
    let intent = IntentRecord {
        id: "intent-1".into(),
        kind: "scheduled".into(),
        status: "active".into(),
        task: "Ask how the deployment went".into(),
        person: Some(PersonId("sam".into())),
        profile: Some(ProfileId("profile-sam".into())),
        conversation: Some(ConversationId("relay:local".into())),
        fire_at: Some(1000),
        condition: None,
        recurrence: None,
        priority: 80,
        dedupe_key: Some("followup:deploy".into()),
        source_action: Some("action-1".into()),
        source_memory: Some(MemoryId("memory-commitment-1".into())),
        created_at: 900,
        updated_at: 900,
        last_fired_at: None,
        owner_approved: true,
    };

    store.create_intent(&intent).await.unwrap();
    let stored = store.get_intent("intent-1").await.unwrap().unwrap();
    assert_eq!(stored.task, "Ask how the deployment went");
    assert!(stored.owner_approved);
    assert_eq!(
        stored.source_memory.as_ref().map(|id| id.0.as_str()),
        Some("memory-commitment-1")
    );

    store
        .update_intent(
            "intent-1",
            &IntentUpdateRecord {
                task: Some("Ask whether the deployment recovered".into()),
                priority: Some(90),
                source_memory: Some(MemoryId("memory-commitment-2".into())),
                owner_approved: Some(false),
                updated_at: 950,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let due = store.due_intents(1000, 10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].priority, 90);
    assert_eq!(due[0].task, "Ask whether the deployment recovered");
    assert!(!due[0].owner_approved);
    assert_eq!(
        due[0].source_memory.as_ref().map(|id| id.0.as_str()),
        Some("memory-commitment-2")
    );

    assert!(store.mark_intent_fired("intent-1", 1001).await.unwrap());
    assert!(!store.mark_intent_fired("intent-1", 1002).await.unwrap());
    assert!(store.due_intents(2000, 10).await.unwrap().is_empty());

    let fired = store.get_intent("intent-1").await.unwrap().unwrap();
    assert_eq!(fired.status, "fired");
    assert_eq!(fired.last_fired_at, Some(1001));
}

#[tokio::test]
async fn intents_can_be_marked_completed_once() {
    let store = test_store();
    store
        .create_intent(&IntentRecord {
            id: "intent-complete".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Close resolved loop".into(),
            person: Some(PersonId("sam".into())),
            profile: None,
            conversation: Some(ConversationId("relay:local".into())),
            fire_at: Some(1000),
            condition: None,
            recurrence: None,
            priority: 50,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: 900,
            updated_at: 900,
            last_fired_at: None,
            owner_approved: false,
        })
        .await
        .unwrap();

    assert!(
        store
            .complete_intent("intent-complete", 1100)
            .await
            .unwrap()
    );
    assert!(
        !store
            .complete_intent("intent-complete", 1200)
            .await
            .unwrap()
    );
    assert!(store.due_intents(2000, 10).await.unwrap().is_empty());

    let completed = store.get_intent("intent-complete").await.unwrap().unwrap();
    assert_eq!(completed.status, "completed");
    assert_eq!(completed.updated_at, 1100);
}

#[tokio::test]
async fn cancelled_intents_are_not_marked_completed() {
    let store = test_store();
    store
        .create_intent(&IntentRecord {
            id: "intent-cancelled".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Cancelled loop".into(),
            person: Some(PersonId("sam".into())),
            profile: None,
            conversation: Some(ConversationId("relay:local".into())),
            fire_at: Some(1000),
            condition: None,
            recurrence: None,
            priority: 50,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: 900,
            updated_at: 900,
            last_fired_at: None,
            owner_approved: false,
        })
        .await
        .unwrap();

    assert!(store.cancel_intent("intent-cancelled", 1000).await.unwrap());
    assert!(
        !store
            .complete_intent("intent-cancelled", 1100)
            .await
            .unwrap()
    );

    let cancelled = store.get_intent("intent-cancelled").await.unwrap().unwrap();
    assert_eq!(cancelled.status, "cancelled");
    assert_eq!(cancelled.updated_at, 1000);
}

#[tokio::test]
async fn recurring_intents_reschedule_when_fired() {
    let store = test_store();
    store
        .create_intent(&IntentRecord {
            id: "intent-recurring".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Check weekly project status".into(),
            person: Some(PersonId("sam".into())),
            profile: None,
            conversation: Some(ConversationId("relay:local".into())),
            fire_at: Some(1000),
            condition: None,
            recurrence: Some("every 2 hours".into()),
            priority: 80,
            dedupe_key: Some("followup:recurring-project-status".into()),
            source_action: Some("action-1".into()),
            source_memory: None,
            created_at: 900,
            updated_at: 900,
            last_fired_at: None,
            owner_approved: false,
        })
        .await
        .unwrap();

    assert_eq!(
        store.due_intents(1000, 10).await.unwrap()[0].id,
        "intent-recurring"
    );
    assert!(
        store
            .mark_intent_fired("intent-recurring", 1001)
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_intent_fired("intent-recurring", 1002)
            .await
            .unwrap()
    );

    let rescheduled = store.get_intent("intent-recurring").await.unwrap().unwrap();
    assert_eq!(rescheduled.status, "active");
    assert_eq!(rescheduled.last_fired_at, Some(1001));
    assert_eq!(rescheduled.fire_at, Some(8200));
    assert!(store.due_intents(8199, 10).await.unwrap().is_empty());
    assert_eq!(
        store.due_intents(8200, 10).await.unwrap()[0].id,
        "intent-recurring"
    );

    assert!(
        store
            .mark_intent_fired("intent-recurring", 20_000)
            .await
            .unwrap()
    );
    let rescheduled = store.get_intent("intent-recurring").await.unwrap().unwrap();
    assert_eq!(rescheduled.status, "active");
    assert_eq!(rescheduled.last_fired_at, Some(20_000));
    assert_eq!(rescheduled.fire_at, Some(22_600));
}

#[tokio::test]
async fn due_intents_coalesce_by_target_per_scan() {
    let store = test_store();
    for (id, person, conversation, dedupe_key, priority) in [
        ("person-high", Some("sam"), Some("relay:sam"), None, 90u8),
        (
            "person-low",
            Some("sam"),
            Some("relay:sam-other"),
            None,
            70u8,
        ),
        ("conversation-high", None, Some("relay:local"), None, 80u8),
        ("conversation-low", None, Some("relay:local"), None, 60u8),
        ("dedupe-high", None, None, Some("followup:deploy"), 50u8),
        ("dedupe-low", None, None, Some("followup:deploy"), 40u8),
        ("unique-a", None, None, None, 30u8),
        ("unique-b", None, None, None, 20u8),
    ] {
        store
            .create_intent(&IntentRecord {
                id: id.into(),
                kind: "scheduled".into(),
                status: "active".into(),
                task: format!("{id} task"),
                person: person.map(|id| PersonId(id.into())),
                profile: None,
                conversation: conversation.map(|id| ConversationId(id.into())),
                fire_at: Some(1000),
                condition: None,
                recurrence: None,
                priority,
                dedupe_key: dedupe_key.map(str::to_string),
                source_action: None,
                source_memory: None,
                created_at: 900,
                updated_at: 900,
                last_fired_at: None,
                owner_approved: false,
            })
            .await
            .unwrap();
    }

    let due = store.due_intents(1000, 10).await.unwrap();
    let ids = due
        .iter()
        .map(|intent| intent.id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        ids,
        vec![
            "person-high",
            "conversation-high",
            "dedupe-high",
            "unique-a",
            "unique-b"
        ]
    );
}

#[tokio::test]
async fn active_intents_for_context_returns_matching_open_loops() {
    let store = test_store();
    for (id, status, person, profile, conversation, fire_at, condition, priority) in [
        (
            "due-person",
            "active",
            Some("sam"),
            None,
            Some("relay:sam"),
            Some(1000),
            None,
            80u8,
        ),
        (
            "future-profile",
            "active",
            None,
            Some("profile-sam"),
            None,
            Some(2000),
            None,
            70u8,
        ),
        (
            "conversation-trigger",
            "active",
            None,
            None,
            Some("relay:local"),
            None,
            Some("next time Sam asks about deployment"),
            90u8,
        ),
        (
            "global-open-loop",
            "active",
            None,
            None,
            None,
            None,
            Some("when any conversation mentions deployment"),
            60u8,
        ),
        (
            "other-person",
            "active",
            Some("alice"),
            None,
            None,
            Some(900),
            None,
            100u8,
        ),
        (
            "cancelled-current",
            "cancelled",
            Some("sam"),
            None,
            None,
            Some(900),
            None,
            100u8,
        ),
    ] {
        store
            .create_intent(&IntentRecord {
                id: id.into(),
                kind: if condition.is_some() {
                    "triggered".into()
                } else {
                    "scheduled".into()
                },
                status: status.into(),
                task: format!("{id} task"),
                person: person.map(|id| PersonId(id.into())),
                profile: profile.map(|id| ProfileId(id.into())),
                conversation: conversation.map(|id| ConversationId(id.into())),
                fire_at,
                condition: condition.map(str::to_string),
                recurrence: None,
                priority,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: 800,
                updated_at: 800,
                last_fired_at: None,
                owner_approved: false,
            })
            .await
            .unwrap();
    }

    let active = store
        .active_intents_for_context(
            Some(&PersonId("sam".into())),
            Some(&ProfileId("profile-sam".into())),
            Some(&ConversationId("relay:local".into())),
            1000,
            10,
        )
        .await
        .unwrap();
    let ids = active
        .iter()
        .map(|intent| intent.id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        ids,
        vec![
            "due-person",
            "conversation-trigger",
            "future-profile",
            "global-open-loop"
        ]
    );
}

#[tokio::test]
async fn event_inbox_persists_due_events_and_fires_once() {
    let store = test_store();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-1".into(),
            kind: "message".into(),
            payload: serde_json::json!({"message_id": "msg-1"}),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: Some("message:relay:msg-1:1".into()),
            created_at: 900,
            updated_at: 900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-duplicate".into(),
            kind: "message".into(),
            payload: serde_json::json!({"message_id": "msg-1"}),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: Some("message:relay:msg-1:1".into()),
            created_at: 901,
            updated_at: 901,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    assert!(store.due_events(999, 10).await.unwrap().is_empty());
    let due = store.due_events(1000, 10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, "event-1");
    assert_eq!(due[0].payload["message_id"], "msg-1");

    assert!(store.mark_event_fired("event-1", 1001).await.unwrap());
    assert!(!store.mark_event_fired("event-1", 1002).await.unwrap());
    assert!(store.due_events(2000, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn due_events_coalesce_message_events_by_conversation_per_scan() {
    let store = test_store();
    for (id, conversation, created_at) in [
        ("event-a-1", "relay:a", 900),
        ("event-a-2", "relay:a", 901),
        ("event-b-1", "relay:b", 902),
    ] {
        store
            .enqueue_event(&EventInboxRecord {
                id: id.into(),
                kind: "message".into(),
                payload: serde_json::json!({
                    "message_id": id.replace("event-", "msg-"),
                    "conversation": conversation,
                }),
                status: "pending".into(),
                due_at: 1000,
                attempts: 0,
                dedupe_key: Some(format!("message:{id}")),
                created_at,
                updated_at: created_at,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();
    }

    let due = store.due_events(1000, 10).await.unwrap();
    let ids = due
        .iter()
        .map(|event| event.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["event-a-1", "event-b-1"]);

    assert!(store.mark_event_fired("event-a-1", 1001).await.unwrap());
    let due = store.due_events(1001, 10).await.unwrap();
    let ids = due
        .iter()
        .map(|event| event.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["event-a-2", "event-b-1"]);
}

#[tokio::test]
async fn event_inbox_failed_events_leave_pending_queue_once() {
    let store = test_store();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-bad".into(),
            kind: "message".into(),
            payload: serde_json::json!({"malformed": true}),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: Some("message:bad".into()),
            created_at: 900,
            updated_at: 900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    assert!(
        store
            .mark_event_failed("event-bad", 1001, Some("malformed test payload"))
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_event_failed("event-bad", 1002, Some("second failure ignored"))
            .await
            .unwrap()
    );
    assert!(store.due_events(2000, 10).await.unwrap().is_empty());

    let conn = store.lock().unwrap();
    let (status, attempts, updated_at, fired_at, last_error): (
        String,
        u32,
        i64,
        Option<i64>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT status, attempts, updated_at, fired_at, last_error FROM event_inbox WHERE id = ?1",
            params!["event-bad"],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(status, "failed");
    assert_eq!(attempts, 1);
    assert_eq!(updated_at, 1001);
    assert_eq!(fired_at, None);
    assert_eq!(last_error.as_deref(), Some("malformed test payload"));
}

#[tokio::test]
async fn event_inbox_surfaces_malformed_payload_rows_for_failure_handling() {
    let store = test_store();
    {
        let conn = store.lock().unwrap();
        conn.execute(
            "INSERT INTO event_inbox (
                id, kind, payload_json, status, due_at, attempts, dedupe_key,
                created_at, updated_at, fired_at, last_error
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                "event-corrupt-payload",
                "message",
                "{not-json",
                "pending",
                1000,
                0_u32,
                Option::<String>::None,
                900,
                900,
                Option::<i64>::None,
                Option::<String>::None,
            ],
        )
        .unwrap();
    }

    let due = store.due_events(1000, 10).await.unwrap();

    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, "event-corrupt-payload");
    assert!(due[0].payload.is_null());
    assert!(
        due[0]
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("malformed event payload json"))
    );
    assert!(
        store
            .mark_event_failed("event-corrupt-payload", 1001, due[0].last_error.as_deref())
            .await
            .unwrap()
    );

    let failed = store.debug_recent_failed_events(10).await.unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].id, "event-corrupt-payload");
    assert!(
        failed[0]
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("malformed event payload json"))
    );
}

#[tokio::test]
async fn event_inbox_lists_pending_events_by_kind_before_due_time() {
    let store = test_store();
    store
        .enqueue_event(&EventInboxRecord {
            id: "message-event".into(),
            kind: "message".into(),
            payload: serde_json::json!({"message_id": "msg-1"}),
            status: "pending".into(),
            due_at: 2000,
            attempts: 0,
            dedupe_key: None,
            created_at: 900,
            updated_at: 900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();
    store
        .enqueue_event(&EventInboxRecord {
            id: "intent-event".into(),
            kind: "intent_fired".into(),
            payload: serde_json::json!({"id": "intent-1"}),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: None,
            created_at: 901,
            updated_at: 901,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    assert!(store.due_events(999, 10).await.unwrap().is_empty());
    let pending = store.pending_events_by_kind("message", 10).await.unwrap();

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "message-event");
    assert_eq!(pending[0].payload["message_id"], "msg-1");
}

#[tokio::test]
async fn snapshots() {
    let store = test_store();
    let snapshot = ActorSnapshot {
        state: ActorState::new(CoreTraits::default()),
        config: GrowthConfig::default(),
        saved_at: 3000,
        last_state_journal_id: Some(7),
    };
    store.save_snapshot(&snapshot).await.unwrap();

    let loaded = store.load_latest_snapshot().await.unwrap().unwrap();
    assert_eq!(loaded.saved_at, 3000);
    assert_eq!(loaded.last_state_journal_id, Some(7));
}

#[tokio::test]
async fn state_journal_records_are_ordered_and_replayable() {
    let store = test_store();
    let first = store
        .append_state_journal("delta", &serde_json::json!({"growth_note": "first"}), 1000)
        .await
        .unwrap();
    let second = store
        .append_state_journal(
            "idle_tick",
            &serde_json::json!({"elapsed_secs": 300.0}),
            1001,
        )
        .await
        .unwrap();

    let records = store.state_journal_after(Some(first), 10).await.unwrap();

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, second);
    assert_eq!(records[0].kind, "idle_tick");
    assert_eq!(records[0].payload["elapsed_secs"], 300.0);
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
            ..Memory::default()
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
            ..Memory::default()
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

#[tokio::test]
async fn recall_filters_memories_due_for_review() {
    let store = test_store();
    for (id, next_review_at, privacy_category) in [
        ("due-public", Some(900), PrivacyCategory::Personal),
        ("future-public", Some(1200), PrivacyCategory::Personal),
        ("due-secret", Some(900), PrivacyCategory::Secret),
        ("not-scheduled", None, PrivacyCategory::Personal),
    ] {
        store
            .store_memory(&Memory {
                id: MemoryId(id.into()),
                kind: MemoryKind::Semantic,
                content: format!("{id} memory"),
                source: MemorySource::Reflection,
                importance: 0.8,
                confidence: 0.8,
                sensitivity: if matches!(privacy_category, PrivacyCategory::Secret) {
                    0.95
                } else {
                    0.0
                },
                privacy_category,
                next_review_at,
                created_at: 1000,
                accessed_at: 1000,
                access_count: 0,
                ..Memory::default()
            })
            .await
            .unwrap();
    }

    let results = store
        .recall(&RecallQuery::by_text("", 10).with_next_review_due(1000))
        .await
        .unwrap();
    assert_eq!(
        results
            .iter()
            .map(|memory| memory.id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["due-public"]
    );

    let results = store
        .recall(
            &RecallQuery::by_text("", 10)
                .with_next_review_due(1000)
                .include_sensitive(),
        )
        .await
        .unwrap();
    let ids = results
        .iter()
        .map(|memory| memory.id.0.as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"due-public"));
    assert!(ids.contains(&"due-secret"));
    assert!(!ids.contains(&"future-public"));
    assert!(!ids.contains(&"not-scheduled"));
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
                source_gateway_id: Some("telegram".into()),
                source_message_id: Some("tg-msg-1".into()),
                sender_external_id: Some("tg-alice".into()),
                reply_external_id: Some("tg-chat".into()),
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

    let relation = SocialRelation {
        person_a: PersonId("p2".into()),
        person_b: PersonId("p1".into()),
        relation: Relation::Parent,
        direction: Relation::Parent.default_direction(),
        confidence: 0.95,
        status: RelationStatus::Confirmed,
        evidence: Some(serde_json::json!({
            "message_ids": ["msg-1"],
            "quote": "my mom"
        })),
        source_kind: RelationSource::OwnerConfirmed,
        asserted_by: Some(PersonId("p1".into())),
        created_at: 1000,
        updated_at: 1000,
    };
    store.upsert_relation(&relation).await.unwrap();

    let rels = store.get_relations(&PersonId("p1".into())).await.unwrap();
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].relation.as_str(), "parent");
    assert_eq!(rels[0].direction.as_str(), "a_to_b");
    assert_eq!(rels[0].confidence, 0.95);
    assert_eq!(rels[0].status, RelationStatus::Confirmed);
    assert_eq!(rels[0].source_kind, RelationSource::OwnerConfirmed);
    assert_eq!(
        rels[0].asserted_by.as_ref().map(|person| person.0.as_str()),
        Some("p1")
    );
    assert_eq!(
        rels[0].evidence.as_ref().unwrap()["message_ids"][0],
        "msg-1"
    );
    assert_eq!(rels[0].created_at, 1000);
    assert_eq!(rels[0].updated_at, 1000);

    let updated = SocialRelation {
        confidence: 0.4,
        status: RelationStatus::Hypothesis,
        evidence: Some(serde_json::json!({"reason": "uncertain"})),
        source_kind: RelationSource::Inferred,
        asserted_by: None,
        updated_at: 1100,
        ..relation.clone()
    };
    store.upsert_relation(&updated).await.unwrap();

    let rels = store.get_relations(&PersonId("p1".into())).await.unwrap();
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].confidence, 0.4);
    assert_eq!(rels[0].status, RelationStatus::Hypothesis);
    assert_eq!(rels[0].source_kind, RelationSource::Inferred);
    assert!(rels[0].asserted_by.is_none());
    assert_eq!(rels[0].evidence.as_ref().unwrap()["reason"], "uncertain");
    assert_eq!(rels[0].created_at, 1000);
    assert_eq!(rels[0].updated_at, 1100);

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
async fn merge_person_context_moves_person_scoped_store_records() {
    let store = test_store();
    let from = PersonId("person-claimant".into());
    let into = PersonId("person-verified".into());
    let other = PersonId("person-other".into());
    store
        .add_person(&sample_person(&from.0, "Claimant"))
        .await
        .unwrap();
    store
        .add_person(&sample_person(&into.0, "Verified"))
        .await
        .unwrap();
    store
        .add_person(&sample_person(&other.0, "Other"))
        .await
        .unwrap();

    store
        .store_memory(&Memory {
            id: MemoryId("memory-person".into()),
            kind: MemoryKind::Semantic,
            content: "Claimant prefers concise updates".into(),
            source: MemorySource::Reflection,
            subjects: vec![MemorySubject::person(
                from.clone(),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    store
        .append_message(
            &ConversationId("relay:claimant".into()),
            Some("relay"),
            None,
            &StoredMessage {
                timestamp: 1000,
                role: MessageRole::User,
                content: "hello".into(),
                identity: None,
                profile: None,
                person: Some(from.clone()),
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-claimant".into()),
                sender_external_id: Some("claimant".into()),
                reply_external_id: Some("claimant".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();
    store
        .create_intent(&IntentRecord {
            id: "intent-person".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Follow up".into(),
            person: Some(from.clone()),
            profile: None,
            conversation: None,
            fire_at: Some(2000),
            condition: None,
            recurrence: None,
            priority: 50,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: 1000,
            updated_at: 1000,
            last_fired_at: None,
            owner_approved: false,
        })
        .await
        .unwrap();
    store
        .add_group(&Group {
            id: GroupId("group-merge".into()),
            name: "Merge Group".into(),
            gateway_id: "relay".into(),
            external_id: "group-merge".into(),
            context: GroupContext::Social,
            members: vec![from.clone()],
        })
        .await
        .unwrap();
    store
        .add_directive(&BehaviorDirective {
            id: "directive-merge".into(),
            scope: DirectiveScope::Person(from.clone()),
            directive: "Use careful wording".into(),
            set_by: into.clone(),
            priority: 10,
            active: true,
            created_at: 1000,
            expires_at: None,
        })
        .await
        .unwrap();
    store
        .upsert_relation(&SocialRelation {
            person_a: from.clone(),
            person_b: other.clone(),
            relation: Relation::Parent,
            direction: Relation::Parent.default_direction(),
            confidence: 0.7,
            status: RelationStatus::Stated,
            evidence: Some(serde_json::json!({"message_id": "msg-claimant"})),
            source_kind: RelationSource::Stated,
            asserted_by: Some(from.clone()),
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();

    store.merge_person_context(&from, &into).await.unwrap();

    let memories = store
        .recall(&RecallQuery::by_text("concise", 10).with_person(into.clone()))
        .await
        .unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].subjects[0].subject_id, into.0);
    let messages = store
        .get_messages(&ConversationId("relay:claimant".into()), 10, None)
        .await
        .unwrap();
    assert_eq!(messages[0].person.as_ref(), Some(&into));
    let intent = store.get_intent("intent-person").await.unwrap().unwrap();
    assert_eq!(intent.person.as_ref(), Some(&into));
    let group = store
        .get_group(&GroupId("group-merge".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group.members, vec![into.clone()]);
    let directives = store
        .get_directives_for_context(&into, &Authority::Default, None)
        .await
        .unwrap();
    assert!(
        directives
            .iter()
            .any(|directive| directive.id == "directive-merge")
    );
    let relations = store.get_relations(&into).await.unwrap();
    assert_eq!(relations.len(), 1);
    assert_eq!(relations[0].person_a, into);
    assert_eq!(relations[0].person_b, other);
    assert_eq!(relations[0].relation.as_str(), "parent");
    assert_eq!(relations[0].direction.as_str(), "a_to_b");
    assert_eq!(relations[0].asserted_by.as_ref(), Some(&into));
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

    let groups = store.debug_groups(10).await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id, GroupId("g1".into()));
    assert!(groups[0].members.contains(&PersonId("p1".into())));
    assert!(groups[0].members.contains(&PersonId("p2".into())));
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
        ..Memory::default()
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
                ..Default::default()
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
            ..Memory::default()
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
async fn actor_subject_recall_does_not_return_profile_identity_memories() {
    let store = test_store();

    store
        .store_memory(&Memory {
            id: MemoryId("profile-name".into()),
            kind: MemoryKind::Semantic,
            content: "Sam said my name is Sam".into(),
            source: MemorySource::Reflection,
            importance: 0.9,
            subjects: vec![MemorySubject::profile(
                ProfileId("profile-sam".into()),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("actor-name".into()),
            kind: MemoryKind::Semantic,
            content: "My name is Pamagotchi".into(),
            source: MemorySource::Reflection,
            importance: 0.9,
            subjects: vec![MemorySubject::actor(Some("self".into()), 1.0)],
            ..Memory::default()
        })
        .await
        .unwrap();

    let actor_memories = store
        .recall(&RecallQuery::by_text("my name", 10).with_actor_subject())
        .await
        .unwrap();

    assert_eq!(actor_memories.len(), 1);
    assert_eq!(actor_memories[0].id.0, "actor-name");
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
            ..Memory::default()
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
