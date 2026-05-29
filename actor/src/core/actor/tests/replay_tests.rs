use super::*;

#[tokio::test]
async fn actor_replays_state_journal_after_snapshot() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    store
        .save_snapshot(&ActorSnapshot {
            state: ActorState::new(CoreTraits::default()),
            config: GrowthConfig::default(),
            saved_at: 1000,
            last_state_journal_id: Some(0),
        })
        .await
        .unwrap();

    let person = PersonId("person-journal".into());
    let delta = Delta {
        relationship_changes: vec![RelationshipChange {
            person: person.clone(),
            trust_delta: 0.0,
            trust_ceiling: None,
            familiarity_delta: 1.0,
            valence_delta: 0.0,
            proactive_consent: None,
            response_cadence: None,
            channel_preference: None,
            interaction: Some(crate::state::RelationshipInteraction::Inbound),
        }],
        ..Delta::default()
    };
    store
        .append_state_journal("delta", &serde_json::to_value(delta).unwrap(), 1001)
        .await
        .unwrap();

    let store_dyn: Arc<dyn Store> = store;
    let actor = ActorBuilder::new(store_dyn, Arc::new(test_router()))
        .build()
        .await
        .unwrap();
    {
        let state = actor.state().read_state();
        let relationship = state.bonds.get(&person).expect("journal replayed bond");
        assert_eq!(relationship.interaction_count, 1);
        assert_eq!(relationship.inbound_count, 1);
        assert!(relationship.familiarity > 0.0);
    }

    actor.shutdown().await.unwrap();
}
#[tokio::test]
async fn actor_replays_relationship_config_and_person_merge_journal_records() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    store
        .save_snapshot(&ActorSnapshot {
            state: ActorState::new(CoreTraits::default()),
            config: GrowthConfig::default(),
            saved_at: 1000,
            last_state_journal_id: Some(0),
        })
        .await
        .unwrap();

    store
        .append_state_journal(
            "relationship_config",
            &serde_json::json!({
                "person_id": "person-claimant",
                "relationship_standing": "trusted",
            }),
            1001,
        )
        .await
        .unwrap();
    store
        .append_state_journal(
            "relationship_config",
            &serde_json::json!({
                "person_id": "person-chosen_human",
                "relationship_standing": "chosen_human",
            }),
            1002,
        )
        .await
        .unwrap();
    store
        .append_state_journal(
            "person_context_merge",
            &serde_json::json!({
                "from_person_id": "person-claimant",
                "into_person_id": "person-chosen_human",
            }),
            1003,
        )
        .await
        .unwrap();

    let store_dyn: Arc<dyn Store> = store;
    let actor = ActorBuilder::new(store_dyn, Arc::new(test_router()))
        .build()
        .await
        .unwrap();
    {
        let state = actor.state().read_state();
        assert!(
            !state
                .bonds
                .contains_key(&PersonId("person-claimant".into()))
        );
        assert_eq!(
            state.bonds[&PersonId("person-chosen_human".into())].relationship_standing,
            crate::state::RelationshipStanding::ChosenHuman
        );
    }

    actor.shutdown().await.unwrap();
}
