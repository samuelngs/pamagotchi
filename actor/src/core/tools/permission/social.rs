use super::STRONG_LIKELY_PERSON_LINK_CONFIDENCE;
use crate::core::tools::SessionContext;
use crate::identity::{
    PersonProfileLink, PersonProfileStatus, RelationSource, RelationStatus, SocialRelation,
};
use crate::state::{Authority, Relationship};
use protocol::{ConversationId, PersonId, ProfileId};
use serde_json::Value;
use std::collections::HashSet;

const CHOSEN_PERSON_SOCIAL_PATH_MIN_CONFIDENCE: f32 = 0.5;
const CHOSEN_PERSON_SOCIAL_PATH_MAX_NODES: usize = 128;

pub(super) async fn person_has_active_profile_context(
    ctx: &SessionContext,
    person: &PersonId,
) -> Result<bool, String> {
    let profiles = ctx
        .store
        .get_profiles_for_person(person)
        .await
        .map_err(|e| format!("Could not verify third-party person target: {e}"))?;
    Ok(profiles
        .into_iter()
        .any(|(_, link)| link.status.is_active_person_context()))
}

pub(crate) async fn person_has_verified_or_strong_profile_context(
    ctx: &SessionContext,
    person: &PersonId,
) -> Result<bool, String> {
    let profiles = ctx
        .store
        .get_profiles_for_person(person)
        .await
        .map_err(|e| format!("Could not verify person profile context: {e}"))?;
    Ok(profiles
        .into_iter()
        .any(|(_, link)| person_link_allows_person_level_update(&link)))
}

pub(super) fn person_link_allows_person_level_update(link: &PersonProfileLink) -> bool {
    match link.status {
        PersonProfileStatus::Verified => true,
        PersonProfileStatus::Likely => link.confidence >= STRONG_LIKELY_PERSON_LINK_CONFIDENCE,
        _ => false,
    }
}

pub(crate) async fn social_relation_targets_current_or_verified(
    args: &Value,
    ctx: &SessionContext,
) -> Result<bool, String> {
    if matches!(ctx.authority, Authority::ChosenPerson) {
        return Ok(true);
    }

    let Some(person_a) = args["person_a"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(|id| PersonId(id.to_string()))
    else {
        return Ok(false);
    };
    let Some(person_b) = args["person_b"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(|id| PersonId(id.to_string()))
    else {
        return Ok(false);
    };
    if social_relation_mentions_chosen_person(ctx, &person_a, &person_b) {
        return Ok(false);
    }

    let Some(current) = ctx
        .messages
        .first()
        .and_then(|message| message.person.clone())
    else {
        return Ok(false);
    };
    if current != person_a && current != person_b {
        return Ok(false);
    }
    person_has_verified_or_strong_profile_context(ctx, &current).await
}

pub(crate) async fn relationship_trust_ceiling(ctx: &SessionContext, person: &PersonId) -> f32 {
    let (authority, current_trust, chosen_people) = {
        let actor = ctx.state.read_state();
        let relationship = actor.bonds.get(person);
        let authority = relationship
            .map(|relationship| relationship.authority.clone())
            .unwrap_or(Authority::Default);
        let current_trust = relationship
            .map(|relationship| relationship.trust)
            .unwrap_or_else(|| Relationship::default().trust);
        let chosen_people = actor
            .bonds
            .iter()
            .filter_map(|(chosen_person, relationship)| {
                matches!(relationship.authority, Authority::ChosenPerson)
                    .then(|| chosen_person.clone())
            })
            .collect::<Vec<_>>();
        (authority, current_trust, chosen_people)
    };

    if !matches!(authority, Authority::Default) {
        return authority.trust_ceiling();
    }
    if chosen_people
        .iter()
        .any(|chosen_person| chosen_person == person)
    {
        return Authority::ChosenPerson.trust_ceiling();
    }
    if has_chosen_person_social_path(ctx, person, &chosen_people).await {
        Authority::Default.trust_ceiling()
    } else {
        current_trust.clamp(0.0, Authority::Default.trust_ceiling())
    }
}

async fn has_chosen_person_social_path(
    ctx: &SessionContext,
    person: &PersonId,
    chosen_people: &[PersonId],
) -> bool {
    let chosen_person_set = chosen_people.iter().cloned().collect::<HashSet<_>>();
    let mut seen = HashSet::from([person.clone()]);
    let mut frontier = vec![person.clone()];

    while !frontier.is_empty() {
        let mut next = Vec::new();
        for current in frontier {
            let Ok(relations) = ctx.store.get_relations(&current).await else {
                return false;
            };
            for relation in relations {
                if !relation_allows_trust_path(&relation) {
                    continue;
                }
                let Some(other) = other_relation_person(&relation, &current) else {
                    continue;
                };
                if chosen_person_set.contains(&other) {
                    return true;
                }
                if seen.insert(other.clone()) {
                    if seen.len() >= CHOSEN_PERSON_SOCIAL_PATH_MAX_NODES {
                        return false;
                    }
                    next.push(other);
                }
            }
        }
        if next.is_empty() {
            return false;
        }
        frontier = next;
    }

    false
}

fn relation_allows_trust_path(relation: &SocialRelation) -> bool {
    matches!(
        relation.status,
        RelationStatus::Confirmed | RelationStatus::Stated
    ) && !matches!(relation.source_kind, RelationSource::Inferred)
        && relation.confidence >= CHOSEN_PERSON_SOCIAL_PATH_MIN_CONFIDENCE
}

fn other_relation_person(relation: &SocialRelation, person: &PersonId) -> Option<PersonId> {
    if &relation.person_a == person {
        Some(relation.person_b.clone())
    } else if &relation.person_b == person {
        Some(relation.person_a.clone())
    } else {
        None
    }
}

fn social_relation_mentions_chosen_person(
    ctx: &SessionContext,
    person_a: &PersonId,
    person_b: &PersonId,
) -> bool {
    let actor = ctx.state.read_state();
    actor.bonds.iter().any(|(person, relationship)| {
        matches!(relationship.authority, Authority::ChosenPerson)
            && (person == person_a || person == person_b)
    })
}

pub(super) async fn profile_has_active_person_context(
    ctx: &SessionContext,
    profile: &ProfileId,
) -> Result<bool, String> {
    let link = ctx
        .store
        .get_person_for_profile(profile)
        .await
        .map_err(|e| format!("Could not verify third-party profile target: {e}"))?;
    Ok(link.is_some_and(|(_, link)| link.status.is_active_person_context()))
}

pub(super) async fn conversation_has_active_person_context(
    ctx: &SessionContext,
    conversation: &ConversationId,
) -> Result<bool, String> {
    let conversations = ctx
        .store
        .list_conversations()
        .await
        .map_err(|e| format!("Could not verify third-party conversation target: {e}"))?;
    let Some(summary) = conversations
        .into_iter()
        .find(|summary| summary.id == *conversation)
    else {
        return Ok(false);
    };
    if let Some(person) = summary.person {
        return person_has_active_profile_context(ctx, &person).await;
    }
    if let Some(profile) = summary.profile {
        return profile_has_active_person_context(ctx, &profile).await;
    }
    Ok(false)
}
