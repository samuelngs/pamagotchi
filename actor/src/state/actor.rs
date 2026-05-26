use super::{AffectState, Belief, CoreTraits, Delta, GrowthConfig, Interest, Relationship};
use protocol::PersonId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Serialize, Deserialize)]
pub struct ActorState {
    pub created_at: i64,
    pub traits: CoreTraits,
    pub beliefs: Vec<Belief>,
    pub bonds: HashMap<PersonId, Relationship>,
    pub interests: Vec<Interest>,
    pub affect: AffectState,
    pub growth_log: Vec<GrowthEvent>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GrowthEvent {
    pub timestamp: i64,
    pub description: String,
    pub person: Option<PersonId>,
}

impl ActorState {
    pub fn new(traits: CoreTraits) -> Self {
        Self {
            created_at: now(),
            traits,
            beliefs: vec![],
            bonds: HashMap::new(),
            interests: vec![],
            affect: AffectState::default(),
            growth_log: vec![],
        }
    }

    pub fn apply_delta(&mut self, delta: &Delta, config: &GrowthConfig) {
        let rate = config.rate_multiplier();
        let damping = 1.0 - config.core_trait_inertia;

        for nudge in &delta.trait_nudges {
            self.traits.nudge(
                &nudge.trait_name,
                nudge.direction * damping * rate,
                &config.trait_floors,
            );
        }

        for change in &delta.belief_changes {
            self.apply_belief_change(change, rate);
        }

        for change in &delta.relationship_changes {
            self.apply_relationship_change(change, rate);
        }

        for topic in &delta.new_interests {
            self.strengthen_interest(topic, rate, delta.triggered_by.as_ref());
        }

        self.affect.shift(&delta.affect_shift);

        if let Some(ref note) = delta.growth_note {
            self.growth_log.push(GrowthEvent {
                timestamp: now(),
                description: note.clone(),
                person: delta.triggered_by.clone(),
            });
            if self.growth_log.len() > 1000 {
                self.growth_log.drain(0..self.growth_log.len() - 1000);
            }
        }
    }

    pub fn tick_idle(&mut self, elapsed_secs: f64) {
        self.affect.mean_revert(elapsed_secs);

        for interest in &mut self.interests {
            interest.decay(elapsed_secs);
        }
        self.interests.retain(|i| i.intensity > 0.01);

        for rel in self.bonds.values_mut() {
            let decay = (-0.00001 * elapsed_secs).exp() as f32;
            rel.emotional_valence *= decay;
        }
    }

    fn apply_belief_change(&mut self, change: &super::BeliefChange, rate: f32) {
        if let Some(belief) = self
            .beliefs
            .iter_mut()
            .find(|b| b.topic == change.topic && b.about == change.about)
        {
            if let Some(ref stance) = change.new_stance {
                belief.stance = stance.clone();
            }
            belief.confidence =
                (belief.confidence + change.confidence_delta * rate).clamp(0.0, 1.0);
            belief.last_challenged = Some(now());
        } else if let Some(ref stance) = change.new_stance {
            self.beliefs.push(Belief {
                topic: change.topic.clone(),
                stance: stance.clone(),
                confidence: (0.5 + change.confidence_delta * rate).clamp(0.0, 1.0),
                formed_at: now(),
                last_challenged: None,
                about: change.about.clone(),
            });
        }
    }

    fn apply_relationship_change(&mut self, change: &super::RelationshipChange, rate: f32) {
        let person = change.person.clone();
        let rel = self
            .bonds
            .entry(person)
            .or_insert_with(Relationship::default);
        rel.trust = (rel.trust + change.trust_delta * rate).clamp(0.0, 1.0);
        rel.familiarity = (rel.familiarity + change.familiarity_delta * rate).clamp(0.0, 1.0);
        rel.emotional_valence =
            (rel.emotional_valence + change.valence_delta * rate).clamp(-1.0, 1.0);
        rel.last_interaction = now();
        rel.interaction_count += 1;
    }

    pub fn set_relationship_config(
        &mut self,
        person: &PersonId,
        authority: Option<super::Authority>,
    ) {
        let rel = self
            .bonds
            .entry(person.clone())
            .or_insert_with(Relationship::default);
        if let Some(a) = authority {
            rel.authority = a;
        }
    }

    pub fn merge_relationship(&mut self, keep: &PersonId, merge: &PersonId) {
        if let Some(merged_rel) = self.bonds.remove(merge) {
            self.bonds.entry(keep.clone()).or_insert(merged_rel);
        }
        let merge_some = Some(merge.clone());
        let keep_some = Some(keep.clone());
        for belief in &mut self.beliefs {
            if belief.about == merge_some {
                belief.about = keep_some.clone();
            }
        }
        let mut i = 0;
        while i < self.beliefs.len() {
            let dominated = (0..i).any(|j| {
                self.beliefs[j].topic == self.beliefs[i].topic
                    && self.beliefs[j].about == self.beliefs[i].about
                    && self.beliefs[j].confidence >= self.beliefs[i].confidence
            });
            if dominated {
                self.beliefs.remove(i);
            } else {
                if let Some(j) = (0..i).find(|&j| {
                    self.beliefs[j].topic == self.beliefs[i].topic
                        && self.beliefs[j].about == self.beliefs[i].about
                }) {
                    self.beliefs.remove(j);
                } else {
                    i += 1;
                }
            }
        }
        for interest in &mut self.interests {
            if interest.origin_person == merge_some {
                interest.origin_person = keep_some.clone();
            }
        }
        for event in &mut self.growth_log {
            if event.person == merge_some {
                event.person = keep_some.clone();
            }
        }
    }

    fn strengthen_interest(&mut self, topic: &str, rate: f32, triggered_by: Option<&PersonId>) {
        if let Some(interest) = self.interests.iter_mut().find(|i| i.topic == topic) {
            interest.intensity = (interest.intensity + 0.1 * rate).clamp(0.0, 1.0);
            interest.last_engaged = now();
        } else {
            self.interests.push(Interest {
                topic: topic.to_string(),
                intensity: 0.3 * rate,
                origin: String::new(),
                origin_person: triggered_by.cloned(),
                last_engaged: now(),
            });
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
