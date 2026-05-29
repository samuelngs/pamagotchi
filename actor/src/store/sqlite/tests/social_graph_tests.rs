use super::*;

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
        source_kind: RelationSource::ChosenHumanConfirmed,
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
    assert_eq!(rels[0].source_kind, RelationSource::ChosenHumanConfirmed);
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
            chosen_human_approved: false,
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
