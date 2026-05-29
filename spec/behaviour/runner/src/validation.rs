use super::json::*;
use super::seed::{SeedRefs, validate_seed};
use super::vocabulary::Vocabulary;
use super::{BehaviourCase, case_paths, load_yaml};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

pub fn load_validated_cases(root: &Path) -> Vec<BehaviourCase> {
    let behaviour_dir = root.join("spec/behaviour");
    let runtime_path = root.join("spec/runtime.yaml");

    validate_runtime(&runtime_path);

    let vocab = Vocabulary::load(&behaviour_dir.join("vocabulary.md"));
    let mut ids = BTreeSet::new();
    let mut paths = case_paths(&behaviour_dir.join("cases"));
    assert!(!paths.is_empty(), "expected behaviour case files");
    paths.sort();

    let mut cases = Vec::new();
    for path in paths {
        let case = load_yaml(&path);
        validate_case(&path, &case, &vocab, &mut ids);
        cases.push(BehaviourCase { path, value: case });
    }
    cases
}

fn validate_runtime(path: &Path) {
    let runtime = load_yaml(path);
    assert_eq!(required_u64(&runtime, "schema_version", path), 1);
    let inference = required_object(&runtime, "default_inference", path);
    assert_eq!(required_str(inference, "id", path), "codex");
    assert_eq!(required_str(inference, "kind", path), "codex");
    assert_string_array_contains_exactly(inference, "capabilities", &["chat", "vision"], path);
    required_object(inference, "options", path);
}

fn validate_case(path: &Path, case: &Value, vocab: &Vocabulary, ids: &mut BTreeSet<String>) {
    validate_case_header(path, case, ids);

    if let Some(runtime) = case.get("runtime") {
        validate_runtime_override(runtime, path);
    }

    let seed_refs = case
        .get("seed")
        .map(|seed| validate_seed(seed, vocab, path))
        .unwrap_or_default();

    validate_scenario(case, path);
    validate_given(case, vocab, &seed_refs, path);
    validate_input(case, &seed_refs, path);
    validate_expected_behavior(case, vocab, path);
    validate_state_expectations(case, vocab, path);
    validate_examples(case, path);
}

fn validate_case_header(path: &Path, case: &Value, ids: &mut BTreeSet<String>) {
    assert_eq!(required_u64(case, "schema_version", path), 1);

    let id = required_str(case, "id", path);
    assert!(
        ids.insert(id.to_string()),
        "duplicate behaviour case id {id} in {}",
        path.display()
    );
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("case path should have a UTF-8 filename");
    assert!(
        filename == format!("{id}.yaml") || filename.starts_with(&format!("{id}-")),
        "case id {id} must match filename {filename}"
    );

    assert_nonempty_str(case, "title", path);
    if let Some(status) = optional_str(case, "status") {
        assert_in(status, &["draft"], "status", path);
    }
    assert_in(
        required_str(case, "priority", path),
        &["p0", "p1", "p2"],
        "priority",
        path,
    );
    assert_nonempty_string_array(case, "tags", path);
}

fn validate_runtime_override(runtime: &Value, path: &Path) {
    let inference = required_object(runtime, "inference", path);
    assert_nonempty_str(inference, "id", path);
    assert_nonempty_str(inference, "kind", path);
    assert_nonempty_string_array(inference, "capabilities", path);
    required_object(inference, "options", path);
}

fn validate_scenario(case: &Value, path: &Path) {
    let scenario = required_object(case, "scenario", path);
    assert_nonempty_str(scenario, "who", path);
    assert_nonempty_str(scenario, "when", path);
    assert_nonempty_str(scenario, "what_happened", path);
}

fn validate_given(case: &Value, vocab: &Vocabulary, refs: &SeedRefs, path: &Path) {
    let given = required_object(case, "given", path);
    validate_optional_enum(
        given,
        "relationship_phase",
        &vocab.relationship_phases,
        path,
    );
    validate_optional_enum(given, "user_comm_style", &vocab.comm_styles, path);

    if let Some(profile_id) = optional_str(given, "current_profile_id") {
        assert!(
            refs.profiles.contains(profile_id),
            "{} given.current_profile_id references unknown profile {profile_id}",
            path.display()
        );
    }
    if let Some(group_id) = optional_str(given, "current_group_id") {
        assert!(
            refs.groups.contains(group_id),
            "{} given.current_group_id references unknown group {group_id}",
            path.display()
        );
    }
    if let Some(person_id) = optional_str(given, "claimed_person_id") {
        assert!(
            refs.people.contains(person_id),
            "{} given.claimed_person_id references unknown person {person_id}",
            path.display()
        );
    }
}

fn validate_input(case: &Value, refs: &SeedRefs, path: &Path) {
    let input = required_object(case, "input", path);
    let messages = required_array(input, "messages", path);
    assert!(
        !messages.is_empty(),
        "{} input.messages must not be empty",
        path.display()
    );
    for message in messages {
        assert_in(
            required_str(message, "role", path),
            &["user", "actor", "system"],
            "input message role",
            path,
        );
        assert_nonempty_str(message, "text", path);
        if let Some(profile_id) = optional_str(message, "profile_id") {
            assert!(
                refs.profiles.contains(profile_id),
                "{} input message references unknown profile {profile_id}",
                path.display()
            );
        }
        if let Some(group_id) = optional_str(message, "group_id") {
            assert!(
                refs.groups.contains(group_id),
                "{} input message references unknown group {group_id}",
                path.display()
            );
        }
    }
}

fn validate_expected_behavior(case: &Value, vocab: &Vocabulary, path: &Path) {
    let expected = required_object(case, "expected_behavior", path);
    validate_string_array_enum(expected, "required_beats", &vocab.required_beats, path);
    validate_string_array_enum(expected, "forbidden_beats", &vocab.forbidden_beats, path);
    validate_cadence(expected, vocab, path);
    validate_string_array_enum(expected, "tone", &vocab.tone_labels, path);
    reject_field(expected, "forbidden_phrases", path);
    validate_freshness(expected, path);
}

fn validate_cadence(expected: &Value, vocab: &Vocabulary, path: &Path) {
    let cadence = required_object(expected, "cadence", path);
    validate_optional_enum(cadence, "mode", &vocab.cadence_modes, path);
    let min_messages = required_u64(cadence, "min_messages", path);
    let max_messages = required_u64(cadence, "max_messages", path);
    assert!(
        min_messages > 0,
        "{} cadence.min_messages must be > 0",
        path.display()
    );
    assert!(
        min_messages <= max_messages,
        "{} cadence min_messages must be <= max_messages",
        path.display()
    );
}

fn validate_freshness(expected: &Value, path: &Path) {
    let Some(freshness) = expected.get("freshness") else {
        return;
    };
    assert!(
        freshness.is_object(),
        "{} expected_behavior.freshness must be an object",
        path.display()
    );

    validate_optional_u64(freshness, "max_repeated_message_occurrences", path);
    validate_optional_u64(freshness, "min_words_per_message", path);
    validate_optional_u64(freshness, "max_words_per_message", path);
    if let Some(min_distinct) = freshness
        .get("min_distinct_sequences")
        .map(|_| required_u64(freshness, "min_distinct_sequences", path))
    {
        assert!(
            min_distinct > 1,
            "{} freshness.min_distinct_sequences must be > 1",
            path.display()
        );
    }
    for key in [
        "max_acceptable_example_message_reuse",
        "max_identity_lookup_messages",
        "identity_lookup_must_be_final",
        "required_any_message_fragments",
        "required_any_message_fragment_groups",
        "forbidden_message_fragments",
        "forbidden_exact_messages",
        "forbidden_words",
        "identity_lookup_markers",
    ] {
        reject_field(freshness, key, path);
    }
}

fn validate_optional_u64(value: &Value, key: &str, path: &Path) {
    if value.get(key).is_some() {
        required_u64(value, key, path);
    }
}

fn validate_optional_bool(value: &Value, key: &str, path: &Path) {
    if let Some(actual) = value.get(key) {
        assert!(
            actual.is_boolean(),
            "{} field {key} must be a boolean",
            path.display()
        );
    }
}

fn reject_field(value: &Value, key: &str, path: &Path) {
    assert!(
        value.get(key).is_none(),
        "{} field {key} is not supported; use semantic beats, examples, cadence, and deterministic state checks instead",
        path.display()
    );
}

fn validate_state_expectations(case: &Value, vocab: &Vocabulary, path: &Path) {
    let Some(state) = case.get("state_expectations") else {
        return;
    };
    assert!(
        state.is_object(),
        "{} state_expectations must be an object",
        path.display()
    );
    if let Some(phase) = optional_str(state, "relationship_phase_after") {
        assert_set_contains(
            &vocab.relationship_phases,
            phase,
            "relationship_phase_after",
            path,
        );
    }
    if let Some(authority) = optional_str(state, "current_profile_authority_after") {
        assert_set_contains(
            &vocab.authorities,
            authority,
            "current_profile_authority_after",
            path,
        );
    }
    if let Some(adoption_state) = optional_str(state, "adoption_state_after") {
        assert_set_contains(
            &vocab.adoption_states,
            adoption_state,
            "adoption_state_after",
            path,
        );
    }
    validate_optional_bool(state, "chosen_human_after", path);
    if let Some(role) = optional_str(state, "bond_role_after") {
        assert_in(
            role,
            &[
                "chosen_human",
                "trusted",
                "default",
                "restricted",
                "blocked",
            ],
            "bond_role_after",
            path,
        );
    }
}

fn validate_examples(case: &Value, path: &Path) {
    let examples = required_object(case, "examples", path);
    for example in required_array(examples, "acceptable", path) {
        validate_example_messages(example, path);
    }
    for example in required_array(examples, "unacceptable", path) {
        validate_example_messages(example, path);
        assert_nonempty_str(example, "reason", path);
    }
}

fn validate_example_messages(example: &Value, path: &Path) {
    let messages = required_array(example, "messages", path);
    assert!(
        !messages.is_empty(),
        "{} example messages must not be empty",
        path.display()
    );
    for message in messages {
        let Some(text) = message.as_str() else {
            panic!("{} example messages must be strings", path.display());
        };
        assert!(
            !text.trim().is_empty(),
            "{} example messages must not be empty",
            path.display()
        );
    }
}
