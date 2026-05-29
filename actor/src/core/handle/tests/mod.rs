use super::*;
use crate::identity::{Relation, RelationSource, RelationStatus};
use crate::state::{CoreTraits, Interest};
use crate::store::{SqliteStore, Store};

#[tokio::test]
async fn relationship_config_updates_shared_state_and_journal() {
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(CoreTraits::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let (tx, rx) = mpsc::channel(1);
    let state_task = StateTask::new(shared.clone(), store.clone(), rx, None);
    let state_join = tokio::spawn(async move {
        state_task.run().await;
    });
    let handle = StateHandle::new(shared.clone(), tx);
    let person = PersonId("person-chosen_human".into());

    handle
        .set_relationship_config(&person, Some(Authority::ChosenHuman))
        .await;

    assert_eq!(
        shared.actor.read().unwrap().bonds[&person].authority,
        Authority::ChosenHuman
    );
    let records = store.state_journal_after(None, 10).await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind, "relationship_config");
    assert_eq!(records[0].payload["person_id"], "person-chosen_human");
    assert_eq!(records[0].payload["authority"], "chosen_human");

    drop(handle);
    state_join.await.unwrap();
}

#[tokio::test]
async fn person_context_merge_updates_shared_state_and_journal() {
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(CoreTraits::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let (tx, rx) = mpsc::channel(1);
    let state_task = StateTask::new(shared.clone(), store.clone(), rx, None);
    let state_join = tokio::spawn(async move {
        state_task.run().await;
    });
    let handle = StateHandle::new(shared.clone(), tx);
    let from = PersonId("person-claimant".into());
    let into = PersonId("person-verified".into());
    {
        let mut actor = shared.actor.write().unwrap();
        actor.set_relationship_config(&from, Some(Authority::Trusted));
    }

    handle.merge_person_context(&from, &into).await;

    let actor = shared.actor.read().unwrap();
    assert!(!actor.bonds.contains_key(&from));
    assert!(actor.bonds.contains_key(&into));
    drop(actor);
    let records = store.state_journal_after(None, 10).await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind, "person_context_merge");
    assert_eq!(records[0].payload["from_person_id"], "person-claimant");
    assert_eq!(records[0].payload["into_person_id"], "person-verified");

    drop(handle);
    state_join.await.unwrap();
}

#[tokio::test]
async fn idle_tick_decays_state_and_journals() {
    let mut actor = ActorState::new(CoreTraits::default());
    actor.affect.valence = 0.9;
    actor.affect.arousal = 0.8;
    actor.affect.dominance = 0.9;
    actor.interests.push(Interest {
        topic: "deployment systems".into(),
        intensity: 0.8,
        origin: "test".into(),
        origin_person: None,
        last_engaged: 1000,
    });
    let shared = Arc::new(SharedState {
        actor: RwLock::new(actor),
        config: RwLock::new(GrowthConfig::default()),
    });
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let (tx, rx) = mpsc::channel(1);
    let state_task = StateTask::new(shared.clone(), store.clone(), rx, None);
    let state_join = tokio::spawn(async move {
        state_task.run().await;
    });
    let handle = StateHandle::new(shared.clone(), tx);

    handle.tick_idle(600.0).await;
    drop(handle);
    state_join.await.unwrap();

    let actor = shared.actor.read().unwrap();
    assert!(actor.affect.valence < 0.9);
    assert!(actor.affect.arousal < 0.8);
    assert!(actor.affect.dominance < 0.9);
    assert_eq!(actor.interests.len(), 1);
    assert!(actor.interests[0].intensity < 0.8);
    drop(actor);

    let records = store.state_journal_after(None, 10).await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind, "idle_tick");
    assert_eq!(records[0].payload["elapsed_secs"], 600.0);
}

#[tokio::test]
async fn state_task_blocks_positive_trust_for_unconnected_default_person() {
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(CoreTraits::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let (_tx, rx) = mpsc::channel(1);
    let mut task = StateTask::new(shared.clone(), store.clone(), rx, None);
    let person = PersonId("person-stranger".into());

    task.apply_batch(vec![StateCommand::Delta(Delta {
        relationship_changes: vec![crate::state::RelationshipChange {
            person: person.clone(),
            trust_delta: 0.05,
            trust_ceiling: None,
            familiarity_delta: 0.0,
            valence_delta: 0.0,
            proactive_consent: None,
            response_cadence: None,
            channel_preference: None,
            interaction: None,
        }],
        ..Delta::default()
    })])
    .await;

    let actor = shared.actor.read().unwrap();
    assert_eq!(actor.bonds[&person].trust, 0.3);
}

#[tokio::test]
async fn state_task_allows_trust_for_chosen_human_connected_social_path() {
    let chosen_human = PersonId("person-chosen_human".into());
    let middle = PersonId("person-middle".into());
    let person = PersonId("person-connected".into());
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(CoreTraits::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    {
        let mut actor = shared.actor.write().unwrap();
        actor.set_relationship_config(&chosen_human, Some(Authority::ChosenHuman));
    }

    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    store
        .upsert_relation(&SocialRelation {
            person_a: chosen_human.clone(),
            person_b: middle.clone(),
            relation: Relation::Friend,
            direction: Relation::Friend.default_direction(),
            confidence: 0.9,
            status: RelationStatus::Confirmed,
            evidence: Some(serde_json::json!({"source": "test"})),
            source_kind: RelationSource::ChosenHumanConfirmed,
            asserted_by: Some(chosen_human.clone()),
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();
    store
        .upsert_relation(&SocialRelation {
            person_a: middle.clone(),
            person_b: person.clone(),
            relation: Relation::Coworker,
            direction: Relation::Coworker.default_direction(),
            confidence: 0.8,
            status: RelationStatus::Stated,
            evidence: Some(serde_json::json!({"source": "test"})),
            source_kind: RelationSource::Stated,
            asserted_by: Some(middle.clone()),
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();

    let (_tx, rx) = mpsc::channel(1);
    let mut task = StateTask::new(shared.clone(), store, rx, None);
    task.apply_batch(vec![StateCommand::Delta(Delta {
        relationship_changes: vec![crate::state::RelationshipChange {
            person: person.clone(),
            trust_delta: 0.05,
            trust_ceiling: None,
            familiarity_delta: 0.0,
            valence_delta: 0.0,
            proactive_consent: None,
            response_cadence: None,
            channel_preference: None,
            interaction: None,
        }],
        ..Delta::default()
    })])
    .await;

    let actor = shared.actor.read().unwrap();
    assert!(actor.bonds[&person].trust > 0.3);
}
