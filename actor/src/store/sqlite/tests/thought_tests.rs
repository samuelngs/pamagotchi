use super::*;

#[tokio::test]
async fn thoughts() {
    let store = test_store();
    store
        .log_thought(&Thought {
            timestamp: 2000,
            kind: ThoughtKind::Reflection,
            content: "Sam seemed stressed".into(),
            importance: 0.9,
            confidence: 0.8,
            action_id: Some("action-1".into()),
            memories_accessed: vec![MemoryId("m1".into())],
            subjects: vec![MemorySubject::profile(
                ProfileId("profile-sam".into()),
                Some("about".into()),
                1.0,
            )],
        })
        .await
        .unwrap();
    store
        .log_thought(&Thought {
            timestamp: 2001,
            kind: ThoughtKind::Planning,
            content: "Unrelated action thought".into(),
            importance: 0.7,
            confidence: 0.7,
            action_id: Some("action-2".into()),
            memories_accessed: vec![],
            subjects: vec![],
        })
        .await
        .unwrap();
    store
        .log_thought(&Thought {
            timestamp: 2002,
            kind: ThoughtKind::Reflection,
            content: "Alice unrelated thought".into(),
            importance: 0.95,
            confidence: 0.95,
            action_id: Some("action-3".into()),
            memories_accessed: vec![],
            subjects: vec![MemorySubject::profile(
                ProfileId("profile-alice".into()),
                Some("about".into()),
                1.0,
            )],
        })
        .await
        .unwrap();

    let thoughts = store.recent_thoughts(5).await.unwrap();
    assert_eq!(thoughts.len(), 3);
    assert_eq!(thoughts[0].content, "Sam seemed stressed");
    assert_eq!(thoughts[0].importance, 0.9);
    assert_eq!(thoughts[0].confidence, 0.8);
    assert_eq!(thoughts[0].action_id.as_deref(), Some("action-1"));
    assert_eq!(thoughts[0].subjects[0].subject_id, "profile-sam");
    let sam_thoughts = store
        .recent_thoughts_for_subject(MemorySubjectType::Profile, "profile-sam", 5)
        .await
        .unwrap();
    assert_eq!(sam_thoughts.len(), 1);
    assert_eq!(sam_thoughts[0].content, "Sam seemed stressed");
    let missing_thoughts = store
        .recent_thoughts_for_subject(MemorySubjectType::Profile, "profile-missing", 5)
        .await
        .unwrap();
    assert!(missing_thoughts.is_empty());
    let action_thoughts = store.thoughts_for_action("action-1", 5).await.unwrap();
    assert_eq!(action_thoughts.len(), 1);
    assert_eq!(action_thoughts[0].content, "Sam seemed stressed");
}

#[tokio::test]
async fn repeated_action_thoughts_are_deduped_and_reinforced() {
    let store = test_store();
    let subjects = vec![MemorySubject::profile(
        ProfileId("profile-sam".into()),
        Some("about".into()),
        1.0,
    )];

    store
        .log_thought(&Thought {
            timestamp: 2000,
            kind: ThoughtKind::Reflection,
            content: "Sam seemed stressed".into(),
            importance: 0.4,
            confidence: 0.5,
            action_id: Some("action-1".into()),
            memories_accessed: vec![MemoryId("memory-a".into())],
            subjects: subjects.clone(),
        })
        .await
        .unwrap();
    store
        .log_thought(&Thought {
            timestamp: 2005,
            kind: ThoughtKind::Reflection,
            content: "  sam   seemed STRESSED  ".into(),
            importance: 0.9,
            confidence: 0.6,
            action_id: Some("action-1".into()),
            memories_accessed: vec![MemoryId("memory-a".into()), MemoryId("memory-b".into())],
            subjects: subjects.clone(),
        })
        .await
        .unwrap();
    store
        .log_thought(&Thought {
            timestamp: 2010,
            kind: ThoughtKind::Reflection,
            content: "Sam seemed stressed".into(),
            importance: 0.7,
            confidence: 0.7,
            action_id: Some("action-2".into()),
            memories_accessed: vec![MemoryId("memory-c".into())],
            subjects,
        })
        .await
        .unwrap();

    let action_one = store.thoughts_for_action("action-1", 10).await.unwrap();
    assert_eq!(action_one.len(), 1);
    assert_eq!(action_one[0].timestamp, 2005);
    assert_eq!(action_one[0].importance, 0.9);
    assert_eq!(action_one[0].confidence, 0.6);
    assert_eq!(
        action_one[0].memories_accessed,
        vec![MemoryId("memory-a".into()), MemoryId("memory-b".into())]
    );

    let all = store.recent_thoughts(10).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].action_id.as_deref(), Some("action-1"));
    assert_eq!(all[1].action_id.as_deref(), Some("action-2"));
}

#[tokio::test]
async fn recent_thoughts_prefer_high_signal_entries() {
    let store = test_store();
    for (idx, content, importance, confidence) in [
        (1, "low signal recent thought", 0.1, 0.9),
        (2, "high confidence useful thought", 0.8, 0.95),
        (3, "important but uncertain thought", 0.8, 0.4),
    ] {
        store
            .log_thought(&Thought {
                timestamp: 2000 + idx,
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

    let thoughts = store.recent_thoughts(2).await.unwrap();
    let contents = thoughts
        .iter()
        .map(|thought| thought.content.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        contents,
        vec![
            "high confidence useful thought",
            "important but uncertain thought"
        ]
    );
}

#[tokio::test]
async fn prune_stale_thoughts_only_removes_old_low_signal_rows() {
    let store = test_store();
    for (idx, timestamp, content, importance, confidence) in [
        (1, 1000, "old low signal thought", 0.1, 0.2),
        (2, 1001, "old important thought", 0.9, 0.2),
        (3, 1002, "old confident thought", 0.2, 0.9),
        (4, 5000, "recent low signal thought", 0.1, 0.2),
    ] {
        store
            .log_thought(&Thought {
                timestamp,
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

    let pruned = store
        .prune_stale_thoughts(2000, 0.3, 0.3, 100)
        .await
        .unwrap();
    assert_eq!(pruned, 1);

    let thoughts = store.recent_thoughts(10).await.unwrap();
    let contents = thoughts
        .iter()
        .map(|thought| thought.content.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        contents,
        vec![
            "old important thought",
            "old confident thought",
            "recent low signal thought"
        ]
    );
}
