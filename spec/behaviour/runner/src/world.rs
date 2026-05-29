use super::BehaviourCase;
use super::json::*;
use actor::identity::{
    ClaimEvidence, ClaimStatus, Group, GroupContext, Identity, IdentityClaim, Person,
    PersonProfileStatus, Profile,
};
use actor::state::{ActorState, AdoptionRitualState, Authority, CoreTraits, Relationship};
use actor::store::{
    Memory, MemoryKind, MemorySource, MemoryStability, MemorySubject, MemorySubjectType,
    MemoryType, PrivacyCategory, SqliteStore, Store, StoredMessage, TruthStatus, VisibilityScope,
};
use protocol::{ConversationId, GroupId, IdentityId, MemoryId, PersonId, ProfileId};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const BASE_TIME: i64 = 1_700_000_000;

pub struct SeededWorld {
    pub store: SqliteStore,
    pub actor: ActorState,
    pub counts: SeedCounts,
    pub contexts: SeedContexts,
}

#[derive(Clone, Debug, Default)]
pub struct SeedCounts {
    pub people: usize,
    pub profiles: usize,
    pub identities: usize,
    pub groups: usize,
    pub memories: usize,
    pub conversations: usize,
    pub conversation_messages: usize,
    pub pending_identity_claims: usize,
}

#[derive(Clone, Debug, Default)]
pub struct SeedContexts {
    pub profiles: BTreeMap<String, SeedProfileContext>,
    pub groups: BTreeMap<String, SeedGroupContext>,
    pub profile_to_conversation: BTreeMap<String, ConversationId>,
    pub group_to_conversation: BTreeMap<String, ConversationId>,
}

#[derive(Clone, Debug)]
pub struct SeedProfileContext {
    pub profile_id: ProfileId,
    pub person_id: PersonId,
    pub identity_id: IdentityId,
    pub gateway_id: String,
    pub external_id: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SeedGroupContext {
    pub gateway_id: String,
    pub external_id: String,
    pub name: String,
}

impl SeedContexts {
    pub fn profile(&self, id: &str) -> Option<&SeedProfileContext> {
        self.profiles.get(id)
    }

    pub fn group(&self, id: &str) -> Option<&SeedGroupContext> {
        self.groups.get(id)
    }

    pub fn conversation_for_profile(&self, id: &str) -> Option<&ConversationId> {
        self.profile_to_conversation.get(id)
    }

    pub fn conversation_for_group(&self, id: &str) -> Option<&ConversationId> {
        self.group_to_conversation.get(id)
    }
}

pub async fn seed_world(case: &BehaviourCase) -> anyhow::Result<SeededWorld> {
    let store = SqliteStore::open_in_memory(4)?;
    let mut actor = ActorState::new(CoreTraits::default());
    let mut contexts = SeedContexts::default();
    let mut counts = SeedCounts::default();
    let Some(seed) = case.value.get("seed") else {
        seed_default_given_context(case, &store, &mut actor, &mut contexts, &mut counts).await?;
        return Ok(SeededWorld {
            store,
            actor,
            counts,
            contexts,
        });
    };

    let mut ids = SeedIds::default();

    seed_people(seed, &store, &mut actor, &mut ids, &mut counts).await?;
    seed_profiles(seed, &store, &mut ids, &mut counts, &mut contexts).await?;
    seed_groups(seed, &store, &ids, &mut counts, &mut contexts).await?;
    seed_memories(seed, &store, &ids, &mut counts).await?;
    seed_conversations(seed, &store, &ids, &mut counts, &mut contexts).await?;
    seed_pending_identity_claims(seed, &store, &ids, &mut counts).await?;

    Ok(SeededWorld {
        store,
        actor,
        counts,
        contexts,
    })
}

#[derive(Default)]
struct SeedIds {
    profile_to_identity: BTreeMap<String, IdentityId>,
    profile_to_person: BTreeMap<String, PersonId>,
}

async fn seed_people(
    seed: &Value,
    store: &SqliteStore,
    actor: &mut ActorState,
    ids: &mut SeedIds,
    counts: &mut SeedCounts,
) -> anyhow::Result<()> {
    for person_value in optional_array(seed, "people", case_path()) {
        let id = PersonId(required_str(person_value, "id", case_path()).to_string());
        let name = optional_string(person_value, "name");
        let comm_style = optional_string(person_value, "comm_style");
        let person = Person {
            id: id.clone(),
            name,
            summary: optional_string(person_value, "summary"),
            comm_style,
            first_seen: BASE_TIME,
            last_seen: BASE_TIME,
        };
        store.add_person(&person).await?;

        let authority = optional_str(person_value, "authority")
            .and_then(Authority::parse)
            .unwrap_or(Authority::Default);
        actor.set_relationship_config(&id, Some(authority.clone()));
        tune_relationship(
            actor,
            &id,
            &authority,
            optional_str(person_value, "relationship_phase"),
        );
        if let Some(adoption_state) =
            optional_str(person_value, "adoption_state").and_then(AdoptionRitualState::parse)
        {
            actor.set_adoption_state(&id, adoption_state, BASE_TIME);
        }
        ids.profile_to_person.entry(id.0.clone()).or_insert(id);
        counts.people += 1;
    }
    Ok(())
}

async fn seed_profiles(
    seed: &Value,
    store: &SqliteStore,
    ids: &mut SeedIds,
    counts: &mut SeedCounts,
    contexts: &mut SeedContexts,
) -> anyhow::Result<()> {
    for profile_value in optional_array(seed, "profiles", case_path()) {
        let profile_id = ProfileId(required_str(profile_value, "id", case_path()).to_string());
        let person_id = PersonId(required_str(profile_value, "person_id", case_path()).to_string());
        let gateway_id = required_str(profile_value, "gateway_id", case_path()).to_string();
        let external_id = required_str(profile_value, "external_id", case_path()).to_string();
        let display_name = optional_string(profile_value, "display_name");
        let identity_id = IdentityId(format!("identity-{}", profile_id.0));

        let identity = Identity {
            id: identity_id.clone(),
            gateway_id: gateway_id.clone(),
            external_id: external_id.clone(),
            display_name: display_name.clone(),
            metadata: Some(json!({
                "source": "behaviour_spec_seed",
                "profile_id": profile_id.0,
            })),
            created_at: BASE_TIME,
            last_seen_at: BASE_TIME,
        };
        let profile = Profile {
            id: profile_id.clone(),
            display_name: display_name.clone(),
            summary: optional_string(profile_value, "summary"),
            comm_style: optional_string(profile_value, "comm_style"),
            first_seen: BASE_TIME,
            last_seen: BASE_TIME,
            created_at: BASE_TIME,
            updated_at: BASE_TIME,
        };

        store.add_identity(&identity).await?;
        store.add_profile(&profile).await?;
        store
            .link_identity_to_profile(
                &identity_id,
                &profile_id,
                1.0,
                Some(&json!({"source": "behaviour_spec_seed"})),
            )
            .await?;
        store
            .attach_profile_to_person(
                &profile_id,
                &person_id,
                profile_status(profile_value),
                1.0,
                Some(&json!({"source": "behaviour_spec_seed"})),
            )
            .await?;

        ids.profile_to_identity
            .insert(profile_id.0.clone(), identity_id.clone());
        ids.profile_to_person
            .insert(profile_id.0.clone(), person_id.clone());
        contexts.profiles.insert(
            profile_id.0.clone(),
            SeedProfileContext {
                profile_id,
                person_id,
                identity_id,
                gateway_id,
                external_id,
                display_name,
            },
        );
        counts.profiles += 1;
        counts.identities += 1;
    }
    Ok(())
}

async fn seed_groups(
    seed: &Value,
    store: &SqliteStore,
    ids: &SeedIds,
    counts: &mut SeedCounts,
    contexts: &mut SeedContexts,
) -> anyhow::Result<()> {
    for group_value in optional_array(seed, "groups", case_path()) {
        let id = GroupId(required_str(group_value, "id", case_path()).to_string());
        let mut members = Vec::new();
        for member in optional_array(group_value, "members", case_path()) {
            let profile_id = required_str(member, "profile_id", case_path());
            if let Some(person_id) = ids.profile_to_person.get(profile_id) {
                members.push(person_id.clone());
            }
        }
        members.sort_by(|a, b| a.0.cmp(&b.0));
        members.dedup_by(|a, b| a.0 == b.0);

        let name = optional_str(group_value, "name")
            .unwrap_or("Behaviour Spec Group")
            .to_string();
        let gateway_id = required_str(group_value, "gateway_id", case_path()).to_string();
        let external_id = optional_str(group_value, "external_id")
            .or_else(|| optional_str(group_value, "id"))
            .unwrap_or("behaviour-spec-group")
            .to_string();

        let group = Group {
            id: id.clone(),
            name: name.clone(),
            gateway_id: gateway_id.clone(),
            external_id: external_id.clone(),
            context: optional_str(group_value, "context")
                .map(GroupContext::parse)
                .unwrap_or_else(|| GroupContext::Custom("behaviour_spec".into())),
            members,
        };
        store.add_group(&group).await?;
        contexts.groups.insert(
            id.0.clone(),
            SeedGroupContext {
                gateway_id,
                external_id,
                name,
            },
        );
        counts.groups += 1;
    }
    Ok(())
}

async fn seed_memories(
    seed: &Value,
    store: &SqliteStore,
    ids: &SeedIds,
    counts: &mut SeedCounts,
) -> anyhow::Result<()> {
    for memory_value in optional_array(seed, "memories", case_path()) {
        let id = required_str(memory_value, "id", case_path());
        let sensitivity = optional_f32(memory_value, "sensitivity").unwrap_or(0.0);
        let privacy_category = optional_str(memory_value, "privacy_category")
            .and_then(PrivacyCategory::parse)
            .unwrap_or_else(|| {
                if sensitivity >= 0.9 {
                    PrivacyCategory::Secret
                } else if sensitivity >= 0.6 {
                    PrivacyCategory::Sensitive
                } else {
                    PrivacyCategory::Personal
                }
            });
        let visibility_scope = optional_str(memory_value, "visibility_scope")
            .and_then(VisibilityScope::parse)
            .unwrap_or_else(|| {
                if privacy_category == PrivacyCategory::Secret {
                    VisibilityScope::ChosenHumanOnly
                } else {
                    VisibilityScope::Profile
                }
            });
        let memory_type = optional_str(memory_value, "memory_type")
            .and_then(MemoryType::parse)
            .unwrap_or(MemoryType::Fact);
        let truth_status = optional_str(memory_value, "truth_status")
            .and_then(TruthStatus::parse)
            .unwrap_or(TruthStatus::Confirmed);
        let kind = optional_str(memory_value, "kind")
            .and_then(MemoryKind::parse)
            .unwrap_or(MemoryKind::Semantic);

        let memory = Memory {
            id: MemoryId(id.to_string()),
            kind,
            memory_type,
            truth_status,
            content: required_str(memory_value, "content", case_path()).to_string(),
            source: MemorySource::External,
            importance: optional_f32(memory_value, "importance").unwrap_or(0.8),
            confidence: optional_f32(memory_value, "confidence").unwrap_or(1.0),
            sensitivity,
            sensitivity_category: optional_string(memory_value, "sensitivity_category"),
            emotional_valence: optional_f32(memory_value, "emotional_valence").unwrap_or(0.0),
            created_at: BASE_TIME,
            accessed_at: BASE_TIME,
            tags: optional_string_array(memory_value, "tags"),
            subjects: vec![memory_subject(memory_value, ids)?],
            evidence: json!({"source": "behaviour_spec_seed"}),
            privacy_category,
            visibility_scope,
            last_confirmed_at: Some(BASE_TIME),
            stability: MemoryStability::Stable,
            ..Memory::default()
        };
        store.store_memory(&memory).await?;
        counts.memories += 1;
    }
    Ok(())
}

async fn seed_conversations(
    seed: &Value,
    store: &SqliteStore,
    ids: &SeedIds,
    counts: &mut SeedCounts,
    contexts: &mut SeedContexts,
) -> anyhow::Result<()> {
    for conversation_value in optional_array(seed, "conversations", case_path()) {
        let conversation_id =
            ConversationId(required_str(conversation_value, "id", case_path()).to_string());
        let profile_id =
            optional_str(conversation_value, "profile_id").map(|id| ProfileId(id.to_string()));
        let person_id =
            optional_str(conversation_value, "person_id").map(|id| PersonId(id.to_string()));
        let group_id =
            optional_str(conversation_value, "group_id").map(|id| GroupId(id.to_string()));
        let identity_id = profile_id
            .as_ref()
            .and_then(|profile| ids.profile_to_identity.get(&profile.0))
            .cloned();
        let gateway_id = optional_str(conversation_value, "gateway_id")
            .map(str::to_string)
            .or_else(|| {
                profile_id
                    .as_ref()
                    .and_then(|profile| contexts.profile(&profile.0))
                    .map(|profile| profile.gateway_id.clone())
            })
            .or_else(|| {
                group_id
                    .as_ref()
                    .and_then(|group| contexts.group(&group.0))
                    .map(|group| group.gateway_id.clone())
            });

        if let Some(profile_id) = &profile_id {
            contexts
                .profile_to_conversation
                .insert(profile_id.0.clone(), conversation_id.clone());
        }
        if let Some(group_id) = &group_id {
            contexts
                .group_to_conversation
                .insert(group_id.0.clone(), conversation_id.clone());
        }

        let messages = optional_array(conversation_value, "messages", case_path());
        if messages.is_empty() {
            let stored = StoredMessage {
                timestamp: BASE_TIME,
                role: actor::store::MessageRole::System,
                content: "behaviour spec conversation seed".into(),
                identity: identity_id.clone(),
                profile: profile_id.clone(),
                person: person_id.clone(),
                source_gateway_id: gateway_id.clone(),
                source_message_id: Some(format!("{}:seed", conversation_id.0)),
                sender_external_id: None,
                reply_external_id: None,
                metadata: json!({"source": "behaviour_spec_seed", "placeholder": true}),
            };
            store
                .append_message(
                    &conversation_id,
                    gateway_id.as_deref(),
                    group_id.as_ref(),
                    &stored,
                )
                .await?;
            counts.conversation_messages += 1;
        } else {
            for (idx, message_value) in messages.iter().enumerate() {
                let stored = StoredMessage {
                    timestamp: BASE_TIME + idx as i64,
                    role: message_role(message_value),
                    content: required_str(message_value, "text", case_path()).to_string(),
                    identity: identity_id.clone(),
                    profile: profile_id.clone(),
                    person: person_id.clone(),
                    source_gateway_id: gateway_id.clone(),
                    source_message_id: Some(format!("{}:seed:{idx}", conversation_id.0)),
                    sender_external_id: None,
                    reply_external_id: None,
                    metadata: json!({"source": "behaviour_spec_seed"}),
                };
                store
                    .append_message(
                        &conversation_id,
                        gateway_id.as_deref(),
                        group_id.as_ref(),
                        &stored,
                    )
                    .await?;
                counts.conversation_messages += 1;
            }
        }
        counts.conversations += 1;
    }
    Ok(())
}

async fn seed_default_given_context(
    case: &BehaviourCase,
    store: &SqliteStore,
    actor: &mut ActorState,
    contexts: &mut SeedContexts,
    counts: &mut SeedCounts,
) -> anyhow::Result<()> {
    let given = required_object(&case.value, "given", &case.path);
    let phase = optional_str(given, "relationship_phase");
    let should_seed = matches!(phase, Some("close" | "familiar" | "newly_bonded"));

    if !should_seed {
        return Ok(());
    }

    let person_id = PersonId("person-default".into());
    let profile_id = ProfileId("profile-default-relay".into());
    let identity_id = IdentityId("identity-profile-default-relay".into());
    let gateway_id = "relay".to_string();
    let external_id = "relay-user".to_string();
    let comm_style = optional_str(given, "user_comm_style")
        .filter(|style| *style != "unknown")
        .map(str::to_string);
    let authority = if phase == Some("identity_uncertain") {
        Authority::Default
    } else {
        Authority::ChosenHuman
    };

    let person = Person {
        id: person_id.clone(),
        name: None,
        summary: None,
        comm_style: comm_style.clone(),
        first_seen: BASE_TIME,
        last_seen: BASE_TIME,
    };
    let profile = Profile {
        id: profile_id.clone(),
        display_name: None,
        summary: None,
        comm_style,
        first_seen: BASE_TIME,
        last_seen: BASE_TIME,
        created_at: BASE_TIME,
        updated_at: BASE_TIME,
    };
    let identity = Identity {
        id: identity_id.clone(),
        gateway_id: gateway_id.clone(),
        external_id: external_id.clone(),
        display_name: None,
        metadata: Some(json!({"source": "behaviour_spec_given"})),
        created_at: BASE_TIME,
        last_seen_at: BASE_TIME,
    };

    store.add_person(&person).await?;
    store.add_profile(&profile).await?;
    store.add_identity(&identity).await?;
    store
        .link_identity_to_profile(
            &identity_id,
            &profile_id,
            1.0,
            Some(&json!({"source": "behaviour_spec_given"})),
        )
        .await?;
    store
        .attach_profile_to_person(
            &profile_id,
            &person_id,
            PersonProfileStatus::Verified,
            1.0,
            Some(&json!({"source": "behaviour_spec_given"})),
        )
        .await?;

    actor.set_relationship_config(&person_id, Some(authority.clone()));
    tune_relationship(actor, &person_id, &authority, phase);
    contexts.profiles.insert(
        profile_id.0.clone(),
        SeedProfileContext {
            profile_id,
            person_id,
            identity_id,
            gateway_id,
            external_id,
            display_name: None,
        },
    );
    counts.people += 1;
    counts.profiles += 1;
    counts.identities += 1;
    Ok(())
}

async fn seed_pending_identity_claims(
    seed: &Value,
    store: &SqliteStore,
    ids: &SeedIds,
    counts: &mut SeedCounts,
) -> anyhow::Result<()> {
    for claim_value in optional_array(seed, "pending_identity_claims", case_path()) {
        let claimant_profile_id = required_str(claim_value, "claimant_profile_id", case_path());
        let Some(claimant) = ids.profile_to_person.get(claimant_profile_id).cloned() else {
            anyhow::bail!("unknown claimant profile {claimant_profile_id}");
        };
        let claim = IdentityClaim {
            id: required_str(claim_value, "id", case_path()).to_string(),
            claimant,
            claimed_person: PersonId(
                required_str(claim_value, "claimed_person_id", case_path()).to_string(),
            ),
            evidence: optional_str(claim_value, "evidence")
                .and_then(ClaimEvidence::parse)
                .unwrap_or(ClaimEvidence::SelfDeclaration),
            reason: optional_string(claim_value, "reason"),
            evidence_json: json!({"source": "behaviour_spec_seed"}),
            confidence: optional_f32(claim_value, "confidence").unwrap_or(0.05),
            status: optional_str(claim_value, "status")
                .and_then(ClaimStatus::parse)
                .unwrap_or(ClaimStatus::Pending),
            created_at: BASE_TIME,
            resolved_at: None,
        };
        store.create_claim(&claim).await?;
        counts.pending_identity_claims += 1;
    }
    Ok(())
}

fn tune_relationship(
    actor: &mut ActorState,
    person: &PersonId,
    authority: &Authority,
    phase: Option<&str>,
) {
    let Some(rel) = actor.bonds.get_mut(person) else {
        return;
    };
    *rel = relationship_for(authority, phase);
}

fn relationship_for(authority: &Authority, phase: Option<&str>) -> Relationship {
    let mut rel = Relationship {
        authority: authority.clone(),
        ..Relationship::default()
    };
    match phase {
        Some("close") => {
            rel.trust = authority.trust_ceiling().min(0.95);
            rel.familiarity = 0.9;
            rel.closeness = 0.85;
            rel.interaction_count = 40;
            rel.inbound_count = 20;
            rel.outbound_count = 20;
        }
        Some("familiar") => {
            rel.trust = authority.trust_ceiling().min(0.8);
            rel.familiarity = 0.7;
            rel.closeness = 0.55;
            rel.interaction_count = 16;
            rel.inbound_count = 8;
            rel.outbound_count = 8;
        }
        Some("newly_bonded") => {
            rel.trust = authority.trust_ceiling().min(0.55);
            rel.familiarity = 0.25;
            rel.closeness = 0.25;
            rel.interaction_count = 2;
            rel.inbound_count = 1;
            rel.outbound_count = 1;
        }
        Some("identity_uncertain") | Some("first_encounter") => {
            rel.trust = authority.trust_ceiling().min(0.25);
            rel.familiarity = 0.05;
            rel.interaction_count = 1;
            rel.inbound_count = 1;
        }
        Some("strained") => {
            rel.trust = authority.trust_ceiling().min(0.35);
            rel.familiarity = 0.5;
            rel.conflict_level = 0.6;
            rel.emotional_valence = -0.3;
            rel.interaction_count = 12;
            rel.inbound_count = 6;
            rel.outbound_count = 6;
        }
        _ => {}
    }
    rel.last_interaction = BASE_TIME;
    rel.last_inbound = if rel.inbound_count > 0 { BASE_TIME } else { 0 };
    rel.last_outbound = if rel.outbound_count > 0 { BASE_TIME } else { 0 };
    rel
}

fn profile_status(value: &Value) -> PersonProfileStatus {
    optional_str(value, "status")
        .and_then(PersonProfileStatus::parse)
        .unwrap_or(PersonProfileStatus::Verified)
}

fn memory_subject(memory_value: &Value, ids: &SeedIds) -> anyhow::Result<MemorySubject> {
    let subject = required_object(memory_value, "subject", case_path());
    let subject_type = required_str(subject, "type", case_path());
    let subject_id = required_str(subject, "id", case_path());
    let role = optional_string(subject, "role");
    let confidence = optional_f32(subject, "confidence").unwrap_or(1.0);
    match subject_type {
        "actor" => Ok(MemorySubject::actor(role, confidence)),
        "identity" => Ok(MemorySubject {
            subject_type: MemorySubjectType::Identity,
            subject_id: subject_id.to_string(),
            role,
            confidence,
        }),
        "profile" => Ok(MemorySubject::profile(
            ProfileId(subject_id.to_string()),
            role,
            confidence,
        )),
        "person" => Ok(MemorySubject::person(
            PersonId(subject_id.to_string()),
            role,
            confidence,
        )),
        "group" => anyhow::bail!(
            "group memory subjects are not supported by the current store schema: {subject_id}"
        ),
        "profile_identity" => {
            let Some(identity) = ids.profile_to_identity.get(subject_id) else {
                anyhow::bail!("unknown profile identity subject {subject_id}");
            };
            Ok(MemorySubject::identity(identity.clone(), role, confidence))
        }
        other => anyhow::bail!("unsupported memory subject type {other}"),
    }
}

fn message_role(value: &Value) -> actor::store::MessageRole {
    match optional_str(value, "role").unwrap_or("user") {
        "user" => actor::store::MessageRole::User,
        "actor" | "assistant" => actor::store::MessageRole::Assistant,
        "system" => actor::store::MessageRole::System,
        "tool" => actor::store::MessageRole::Tool,
        other => panic!("unsupported seeded message role {other}"),
    }
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    optional_str(value, key).map(str::to_string)
}

fn optional_string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn optional_f32(value: &Value, key: &str) -> Option<f32> {
    value.get(key).and_then(Value::as_f64).map(|v| v as f32)
}

fn case_path() -> &'static std::path::Path {
    std::path::Path::new("behaviour case seed")
}
