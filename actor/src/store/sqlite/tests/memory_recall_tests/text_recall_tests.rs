use super::*;

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
