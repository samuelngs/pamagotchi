use super::json::*;
use super::vocabulary::Vocabulary;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Default)]
pub struct SeedRefs {
    pub people: BTreeSet<String>,
    pub profiles: BTreeSet<String>,
    pub groups: BTreeSet<String>,
    pub memories: BTreeSet<String>,
    pub conversations: BTreeSet<String>,
    pub pending_identity_claims: BTreeSet<String>,
}

pub fn validate_seed(seed: &Value, vocab: &Vocabulary, path: &Path) -> SeedRefs {
    let mut refs = SeedRefs::default();

    validate_people(seed, vocab, path, &mut refs);
    validate_profiles(seed, path, &mut refs);
    validate_groups(seed, path, &mut refs);
    validate_memories(seed, vocab, path, &mut refs);
    validate_conversations(seed, path, &mut refs);
    validate_pending_identity_claims(seed, path, &mut refs);

    refs
}

fn validate_people(seed: &Value, vocab: &Vocabulary, path: &Path, refs: &mut SeedRefs) {
    for person in optional_array(seed, "people", path) {
        let id = required_str(person, "id", path);
        assert_unique(&mut refs.people, id, "person", path);
        validate_optional_enum(person, "authority", &vocab.authorities, path);
        validate_optional_enum(
            person,
            "relationship_phase",
            &vocab.relationship_phases,
            path,
        );
        validate_optional_enum(person, "comm_style", &vocab.comm_styles, path);
    }
}

fn validate_profiles(seed: &Value, path: &Path, refs: &mut SeedRefs) {
    for profile in optional_array(seed, "profiles", path) {
        let id = required_str(profile, "id", path);
        assert_unique(&mut refs.profiles, id, "profile", path);
        let person_id = required_str(profile, "person_id", path);
        assert!(
            refs.people.contains(person_id),
            "{} profile {id} references unknown person {person_id}",
            path.display()
        );
        assert_nonempty_str(profile, "gateway_id", path);
        assert_nonempty_str(profile, "external_id", path);
    }
}

fn validate_groups(seed: &Value, path: &Path, refs: &mut SeedRefs) {
    for group in optional_array(seed, "groups", path) {
        let id = required_str(group, "id", path);
        assert_unique(&mut refs.groups, id, "group", path);
        assert_nonempty_str(group, "gateway_id", path);
        for member in optional_array(group, "members", path) {
            let profile_id = required_str(member, "profile_id", path);
            assert!(
                refs.profiles.contains(profile_id),
                "{} group {id} references unknown member profile {profile_id}",
                path.display()
            );
        }
    }
}

fn validate_memories(seed: &Value, vocab: &Vocabulary, path: &Path, refs: &mut SeedRefs) {
    for memory in optional_array(seed, "memories", path) {
        let id = required_str(memory, "id", path);
        assert_unique(&mut refs.memories, id, "memory", path);
        assert_nonempty_str(memory, "content", path);
        validate_memory_subject(memory, id, refs, path);
        validate_optional_enum(memory, "visibility_scope", &vocab.visibility_scopes, path);
    }
}

fn validate_memory_subject(memory: &Value, memory_id: &str, refs: &SeedRefs, path: &Path) {
    let subject = required_object(memory, "subject", path);
    let subject_type = required_str(subject, "type", path);
    let subject_id = required_str(subject, "id", path);
    match subject_type {
        "person" => assert!(
            refs.people.contains(subject_id),
            "{} memory {memory_id} references unknown person {subject_id}",
            path.display()
        ),
        "profile" => assert!(
            refs.profiles.contains(subject_id),
            "{} memory {memory_id} references unknown profile {subject_id}",
            path.display()
        ),
        "group" => assert!(
            refs.groups.contains(subject_id),
            "{} memory {memory_id} references unknown group {subject_id}",
            path.display()
        ),
        "actor" | "identity" => {}
        other => panic!(
            "{} memory {memory_id} has unsupported subject type {other}",
            path.display()
        ),
    }
}

fn validate_conversations(seed: &Value, path: &Path, refs: &mut SeedRefs) {
    for conversation in optional_array(seed, "conversations", path) {
        let id = required_str(conversation, "id", path);
        assert_unique(&mut refs.conversations, id, "conversation", path);
        if let Some(profile_id) = optional_str(conversation, "profile_id") {
            assert!(
                refs.profiles.contains(profile_id),
                "{} conversation {id} references unknown profile {profile_id}",
                path.display()
            );
        }
        if let Some(person_id) = optional_str(conversation, "person_id") {
            assert!(
                refs.people.contains(person_id),
                "{} conversation {id} references unknown person {person_id}",
                path.display()
            );
        }
        if let Some(group_id) = optional_str(conversation, "group_id") {
            assert!(
                refs.groups.contains(group_id),
                "{} conversation {id} references unknown group {group_id}",
                path.display()
            );
        }
    }
}

fn validate_pending_identity_claims(seed: &Value, path: &Path, refs: &mut SeedRefs) {
    for claim in optional_array(seed, "pending_identity_claims", path) {
        let id = required_str(claim, "id", path);
        assert_unique(
            &mut refs.pending_identity_claims,
            id,
            "pending identity claim",
            path,
        );
        let claimant_profile_id = required_str(claim, "claimant_profile_id", path);
        assert!(
            refs.profiles.contains(claimant_profile_id),
            "{} identity claim {id} references unknown claimant profile {claimant_profile_id}",
            path.display()
        );
        let claimed_person_id = required_str(claim, "claimed_person_id", path);
        assert!(
            refs.people.contains(claimed_person_id),
            "{} identity claim {id} references unknown claimed person {claimed_person_id}",
            path.display()
        );
    }
}
