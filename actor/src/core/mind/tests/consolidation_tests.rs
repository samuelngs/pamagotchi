use super::*;

#[tokio::test]
async fn consolidation_due_spawns_consolidate_action() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store);

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::ConsolidationDue,
        )
        .await;

    match decision {
        MindDecision::Spawn(action) => {
            assert!(matches!(action.kind, ActionKind::Consolidate));
            assert_eq!(action.priority, ActionKind::Consolidate.default_priority());
        }
        _ => panic!("expected consolidation action"),
    }
}
#[tokio::test]
async fn at_capacity_defers_consolidation_due_instead_of_dropping() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store);
    fill_capacity_with_running_responses(&mut mind);

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::ConsolidationDue,
        )
        .await;

    match decision {
        MindDecision::DeferConsolidation(delay_secs) => {
            assert_eq!(delay_secs, 300);
        }
        _ => panic!("expected deferred consolidation at capacity"),
    }
}
#[tokio::test]
async fn deferred_consolidation_is_persisted_for_scheduler_retry() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store.clone());

    mind.execute_decision(MindDecision::DeferConsolidation(300))
        .await;

    let pending = store
        .pending_events_by_kind("consolidation_due", 10)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].dedupe_key.as_deref(), Some("consolidation-due"));
    assert_eq!(pending[0].attempts, 0);
    assert_eq!(mind.metrics.snapshot().events_deferred, 1);
}
#[tokio::test]
async fn consolidation_prunes_stale_low_signal_thoughts() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let now = recent_timestamp();
    for (idx, age_secs, content, importance, confidence) in [
        (
            1,
            STALE_THOUGHT_SECS + 10,
            "old low signal thought",
            0.1,
            0.2,
        ),
        (
            2,
            STALE_THOUGHT_SECS + 10,
            "old important thought",
            0.9,
            0.2,
        ),
        (3, 10, "recent low signal thought", 0.1, 0.2),
    ] {
        store
            .log_thought(&Thought {
                timestamp: now - age_secs,
                kind: ThoughtKind::Observation,
                content: content.into(),
                importance,
                confidence,
                action_id: Some(format!("action-{idx}")),
                memories_accessed: vec![],
                subjects: vec![],
            })
            .await
            .unwrap();
    }

    mind.prune_stale_thoughts(now).await;

    let thoughts = store.recent_thoughts(10).await.unwrap();
    let contents = thoughts
        .iter()
        .map(|thought| thought.content.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        contents,
        vec!["old important thought", "recent low signal thought"]
    );
    assert_eq!(mind.metrics.snapshot().thoughts_pruned, 1);
}
#[tokio::test]
async fn consolidation_prunes_stale_low_signal_memories() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let now = recent_timestamp();
    store
        .store_memory(&Memory {
            id: MemoryId("expired-memory".into()),
            kind: MemoryKind::Semantic,
            content: "Temporary low-signal note".into(),
            source: MemorySource::Reflection,
            importance: 0.1,
            confidence: 0.2,
            sensitivity: 0.1,
            created_at: now - STALE_MEMORY_SECS,
            accessed_at: now - STALE_MEMORY_SECS,
            expires_at: Some(now - 1),
            ..Memory::default()
        })
        .await
        .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("important-expired-memory".into()),
            kind: MemoryKind::Semantic,
            content: "Important expired note".into(),
            source: MemorySource::Reflection,
            importance: 0.9,
            confidence: 0.2,
            sensitivity: 0.1,
            created_at: now - STALE_MEMORY_SECS,
            accessed_at: now - STALE_MEMORY_SECS,
            expires_at: Some(now - 1),
            ..Memory::default()
        })
        .await
        .unwrap();

    mind.prune_stale_memories(now).await;

    assert!(
        store
            .get_memory(&MemoryId("expired-memory".into()))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .get_memory(&MemoryId("important-expired-memory".into()))
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(mind.metrics.snapshot().memories_pruned, 1);
}
