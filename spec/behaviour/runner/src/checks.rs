use super::BehaviourCase;
use super::capture::CapturedOutbound;
use super::json::{optional_array, required_array, required_object, required_str, required_u64};
use serde_json::Value;
use std::collections::BTreeSet;

pub struct OutputChecks {
    pub cadence: CheckOutcome,
    pub forbidden_phrases: CheckOutcome,
    pub freshness: CheckOutcome,
}

pub struct CheckOutcome {
    pub passed: bool,
    pub detail: String,
}

impl CheckOutcome {
    fn pass(detail: impl Into<String>) -> Self {
        Self {
            passed: true,
            detail: detail.into(),
        }
    }

    fn fail(detail: impl Into<String>) -> Self {
        Self {
            passed: false,
            detail: detail.into(),
        }
    }
}

pub fn evaluate_output(
    case: &BehaviourCase,
    output: &[CapturedOutbound],
    timed_out: bool,
) -> OutputChecks {
    OutputChecks {
        cadence: check_cadence(case, output, timed_out),
        forbidden_phrases: check_forbidden_phrases(case, output),
        freshness: check_freshness(case, output),
    }
}

impl OutputChecks {
    pub fn passed(&self) -> bool {
        self.cadence.passed && self.forbidden_phrases.passed && self.freshness.passed
    }
}

pub fn evaluate_repeated_outputs(
    case: &BehaviourCase,
    runs: &[Vec<CapturedOutbound>],
) -> CheckOutcome {
    let expected = required_object(&case.value, "expected_behavior", &case.path);
    let Some(freshness) = optional_object(expected, "freshness", &case.path) else {
        return CheckOutcome::pass("none configured");
    };
    let mut details = Vec::new();
    let mut failures = Vec::new();

    if let Some(min_distinct) = optional_u64(freshness, "min_distinct_sequences", &case.path) {
        let min_distinct = min_distinct as usize;
        if runs.len() < min_distinct {
            details.push(format!(
                "skipped distinct sequences, repeat count {} is below min_distinct_sequences {min_distinct}",
                runs.len()
            ));
        } else {
            let distinct = runs
                .iter()
                .map(|run| normalized_sequence(run))
                .collect::<BTreeSet<_>>();
            let count = distinct.len();
            details.push(format!(
                "got {count} distinct sequences across {} runs, expected at least {min_distinct}",
                runs.len()
            ));
            if count < min_distinct {
                failures.push(format!(
                    "got {count} distinct sequences across {} runs, expected at least {min_distinct}",
                    runs.len()
                ));
            }
        }
    }

    if let Some(max_repeated) =
        optional_u64(freshness, "max_repeated_message_occurrences", &case.path)
    {
        let max_repeated = max_repeated as usize;
        let repeated = repeated_messages(runs, max_repeated);
        details.push(format!(
            "repeated messages over {} occurrences {}",
            max_repeated,
            if repeated.is_empty() {
                "clear".to_string()
            } else {
                repeated.join(" | ")
            }
        ));
        if !repeated.is_empty() {
            failures.push(format!(
                "messages repeated more than {max_repeated} times: {}",
                repeated.join(" | ")
            ));
        }
    }

    if failures.is_empty() {
        CheckOutcome::pass(if details.is_empty() {
            "no repeated-output checks configured".to_string()
        } else {
            details.join("; ")
        })
    } else {
        CheckOutcome::fail(failures.join("; "))
    }
}

fn repeated_messages(runs: &[Vec<CapturedOutbound>], max: usize) -> Vec<String> {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for normalized in runs
        .iter()
        .flat_map(|run| run.iter().map(|message| normalize_words(&message.content)))
        .filter(|message| !message.is_empty())
    {
        *counts.entry(normalized).or_default() += 1;
    }
    counts
        .into_iter()
        .filter_map(|(message, count)| (count > max).then(|| format!("{count}x {message}")))
        .collect()
}

fn check_cadence(
    case: &BehaviourCase,
    output: &[CapturedOutbound],
    timed_out: bool,
) -> CheckOutcome {
    let expected = required_object(&case.value, "expected_behavior", &case.path);
    let cadence = required_object(expected, "cadence", &case.path);
    let mode = required_str(cadence, "mode", &case.path);
    let min = required_u64(cadence, "min_messages", &case.path) as usize;
    let max = required_u64(cadence, "max_messages", &case.path) as usize;
    let count = output.len();

    if timed_out {
        return CheckOutcome::fail(format!(
            "timed out after {count} messages, expected {mode} {min}..{max}"
        ));
    }
    if count < min || count > max {
        return CheckOutcome::fail(format!(
            "got {count} messages, expected {mode} {min}..{max}"
        ));
    }
    CheckOutcome::pass(format!(
        "got {count} messages, expected {mode} {min}..{max}"
    ))
}

fn check_forbidden_phrases(case: &BehaviourCase, output: &[CapturedOutbound]) -> CheckOutcome {
    let expected = required_object(&case.value, "expected_behavior", &case.path);
    let configured = optional_array(expected, "forbidden_phrases", &case.path);
    if configured.is_empty() {
        return CheckOutcome::pass("none configured");
    }
    let combined = output
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    let hits = configured
        .iter()
        .filter_map(|phrase| {
            let phrase = phrase.as_str().unwrap_or_else(|| {
                panic!(
                    "{} forbidden_phrases must contain strings",
                    case.path.display()
                )
            });
            combined
                .contains(&phrase.to_ascii_lowercase())
                .then(|| phrase.to_string())
        })
        .collect::<Vec<_>>();

    if hits.is_empty() {
        CheckOutcome::pass("no forbidden phrases found")
    } else {
        CheckOutcome::fail(format!("found {}", hits.join(", ")))
    }
}

fn check_freshness(case: &BehaviourCase, output: &[CapturedOutbound]) -> CheckOutcome {
    let expected = required_object(&case.value, "expected_behavior", &case.path);
    let Some(freshness) = optional_object(expected, "freshness", &case.path) else {
        return CheckOutcome::pass("none configured");
    };

    let mut details = Vec::new();
    let mut failures = Vec::new();

    if let Some(max_reuse) = optional_u64(
        freshness,
        "max_acceptable_example_message_reuse",
        &case.path,
    ) {
        let max_reuse = max_reuse as usize;
        let acceptable = acceptable_example_messages(case);
        let reused = output
            .iter()
            .filter_map(|message| {
                let normalized = normalize_words(&message.content);
                acceptable
                    .contains(&normalized)
                    .then(|| message.content.clone())
            })
            .collect::<Vec<_>>();
        details.push(format!(
            "acceptable-example reuse {}/{}",
            reused.len(),
            max_reuse
        ));
        if reused.len() > max_reuse {
            failures.push(format!(
                "reused acceptable example messages: {}",
                reused.join(" | ")
            ));
        }
    }

    let required_any_fragments =
        optional_string_array(freshness, "required_any_message_fragments", &case.path);
    if !required_any_fragments.is_empty() {
        let combined = normalize_words(
            &output
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let hits = required_any_fragments
            .iter()
            .filter(|fragment| combined.contains(&normalize_words(fragment)))
            .cloned()
            .collect::<Vec<_>>();
        details.push(format!(
            "required fresh fragment {}",
            if hits.is_empty() {
                "missing".to_string()
            } else {
                hits.join(", ")
            }
        ));
        if hits.is_empty() {
            failures.push(format!(
                "missing one required fragment from: {}",
                required_any_fragments.join(", ")
            ));
        }
    }

    let required_fragment_groups = optional_string_array_groups(
        freshness,
        "required_any_message_fragment_groups",
        &case.path,
    );
    if !required_fragment_groups.is_empty() {
        let combined = normalize_words(
            &output
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        for (idx, group) in required_fragment_groups.iter().enumerate() {
            let hits = group
                .iter()
                .filter(|fragment| combined.contains(&normalize_words(fragment)))
                .cloned()
                .collect::<Vec<_>>();
            details.push(format!(
                "required fragment group {} {}",
                idx + 1,
                if hits.is_empty() {
                    "missing".to_string()
                } else {
                    hits.join(", ")
                }
            ));
            if hits.is_empty() {
                failures.push(format!(
                    "missing required fragment group {}: one of {}",
                    idx + 1,
                    group.join(", ")
                ));
            }
        }
    }

    let forbidden_fragments =
        optional_string_array(freshness, "forbidden_message_fragments", &case.path);
    if !forbidden_fragments.is_empty() {
        let combined = normalize_words(
            &output
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let hits = forbidden_fragments
            .iter()
            .filter(|fragment| combined.contains(&normalize_words(fragment)))
            .cloned()
            .collect::<Vec<_>>();
        details.push(format!(
            "forbidden fresh fragments {}",
            if hits.is_empty() {
                "clear".to_string()
            } else {
                hits.join(", ")
            }
        ));
        if !hits.is_empty() {
            failures.push(format!("found stale fragments: {}", hits.join(", ")));
        }
    }

    let forbidden_exact = optional_string_array(freshness, "forbidden_exact_messages", &case.path);
    if !forbidden_exact.is_empty() {
        let output_messages = output
            .iter()
            .map(|message| normalize_words(&message.content))
            .collect::<BTreeSet<_>>();
        let hits = forbidden_exact
            .iter()
            .filter(|message| output_messages.contains(&normalize_words(message)))
            .cloned()
            .collect::<Vec<_>>();
        details.push(format!(
            "forbidden exact messages {}",
            if hits.is_empty() {
                "clear".to_string()
            } else {
                hits.join(", ")
            }
        ));
        if !hits.is_empty() {
            failures.push(format!(
                "found forbidden exact messages: {}",
                hits.join(", ")
            ));
        }
    }

    let forbidden_words = optional_string_array(freshness, "forbidden_words", &case.path);
    if !forbidden_words.is_empty() {
        let words = output
            .iter()
            .flat_map(|message| {
                normalize_words(&message.content)
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeSet<_>>();
        let hits = forbidden_words
            .iter()
            .filter(|word| words.contains(&normalize_words(word)))
            .cloned()
            .collect::<Vec<_>>();
        details.push(format!(
            "forbidden words {}",
            if hits.is_empty() {
                "clear".to_string()
            } else {
                hits.join(", ")
            }
        ));
        if !hits.is_empty() {
            failures.push(format!("found forbidden words: {}", hits.join(", ")));
        }
    }

    if let Some(max_identity) = optional_u64(freshness, "max_identity_lookup_messages", &case.path)
    {
        let markers = optional_string_array(freshness, "identity_lookup_markers", &case.path);
        let identity_messages = output
            .iter()
            .filter(|message| message_has_marker(&message.content, &markers))
            .map(|message| message.content.clone())
            .collect::<Vec<_>>();
        let max_identity = max_identity as usize;
        details.push(format!(
            "identity lookup messages {}/{}",
            identity_messages.len(),
            max_identity
        ));
        if identity_messages.len() > max_identity {
            failures.push(format!(
                "too many identity lookup messages: {}",
                identity_messages.join(" | ")
            ));
        }
    }

    if optional_bool(freshness, "identity_lookup_must_be_final", &case.path).unwrap_or(false) {
        let markers = optional_string_array(freshness, "identity_lookup_markers", &case.path);
        let last_identity_idx = output
            .iter()
            .rposition(|message| message_has_marker(&message.content, &markers));
        match last_identity_idx {
            Some(idx) if idx + 1 == output.len() => {
                details.push("identity lookup final".to_string());
            }
            Some(idx) => {
                failures.push(format!(
                    "identity lookup message {} was followed by another message",
                    idx + 1
                ));
            }
            None => {
                failures.push("no identity lookup message found".to_string());
            }
        }
    }

    if let Some(min_words) = optional_u64(freshness, "min_words_per_message", &case.path) {
        let min_words = min_words as usize;
        let underlong = output
            .iter()
            .filter_map(|message| {
                let count = word_count(&message.content);
                (count < min_words).then(|| format!("{} words: {}", count, message.content))
            })
            .collect::<Vec<_>>();
        details.push(format!(
            "message length min {} words{}",
            min_words,
            if underlong.is_empty() {
                " clear".to_string()
            } else {
                format!(" missed by {}", underlong.len())
            }
        ));
        if !underlong.is_empty() {
            failures.push(format!("underlong messages: {}", underlong.join(" | ")));
        }
    }

    if let Some(max_words) = optional_u64(freshness, "max_words_per_message", &case.path) {
        let max_words = max_words as usize;
        let overlong = output
            .iter()
            .filter_map(|message| {
                let count = word_count(&message.content);
                (count > max_words).then(|| format!("{} words: {}", count, message.content))
            })
            .collect::<Vec<_>>();
        details.push(format!(
            "message length max {} words{}",
            max_words,
            if overlong.is_empty() {
                " clear".to_string()
            } else {
                format!(" exceeded by {}", overlong.len())
            }
        ));
        if !overlong.is_empty() {
            failures.push(format!("overlong messages: {}", overlong.join(" | ")));
        }
    }

    if failures.is_empty() {
        CheckOutcome::pass(details.join("; "))
    } else {
        CheckOutcome::fail(failures.join("; "))
    }
}

fn acceptable_example_messages(case: &BehaviourCase) -> BTreeSet<String> {
    let examples = required_object(&case.value, "examples", &case.path);
    required_array(examples, "acceptable", &case.path)
        .iter()
        .flat_map(|example| required_array(example, "messages", &case.path))
        .map(|message| {
            normalize_words(message.as_str().unwrap_or_else(|| {
                panic!(
                    "{} acceptable example messages must be strings",
                    case.path.display()
                )
            }))
        })
        .collect()
}

fn message_has_marker(message: &str, markers: &[String]) -> bool {
    let message = normalize_words(message);
    markers.iter().any(|marker| {
        let marker = normalize_words(marker);
        if marker.contains(' ') {
            message.contains(&marker)
        } else {
            message.split_whitespace().any(|word| word == marker)
        }
    })
}

fn normalized_sequence(output: &[CapturedOutbound]) -> String {
    output
        .iter()
        .map(|message| normalize_words(&message.content))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn normalize_words(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut last_was_space = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                normalized.push(lower);
            }
            last_was_space = false;
        } else if !last_was_space {
            normalized.push(' ');
            last_was_space = true;
        }
    }
    normalized.trim().to_string()
}

fn word_count(text: &str) -> usize {
    normalize_words(text).split_whitespace().count()
}

fn optional_object<'a>(value: &'a Value, key: &str, path: &std::path::Path) -> Option<&'a Value> {
    match value.get(key) {
        Some(object) if object.is_object() => Some(object),
        Some(_) => panic!("{} field {key} must be an object", path.display()),
        None => None,
    }
}

fn optional_u64(value: &Value, key: &str, path: &std::path::Path) -> Option<u64> {
    match value.get(key) {
        Some(number) => number
            .as_u64()
            .or_else(|| panic!("{} field {key} must be an unsigned integer", path.display())),
        None => None,
    }
}

fn optional_bool(value: &Value, key: &str, path: &std::path::Path) -> Option<bool> {
    match value.get(key) {
        Some(boolean) => boolean
            .as_bool()
            .or_else(|| panic!("{} field {key} must be a boolean", path.display())),
        None => None,
    }
}

fn optional_string_array(value: &Value, key: &str, path: &std::path::Path) -> Vec<String> {
    optional_array(value, key, path)
        .iter()
        .map(|item| {
            item.as_str()
                .unwrap_or_else(|| panic!("{} field {key} must contain strings", path.display()))
                .to_string()
        })
        .collect()
}

fn optional_string_array_groups(
    value: &Value,
    key: &str,
    path: &std::path::Path,
) -> Vec<Vec<String>> {
    optional_array(value, key, path)
        .iter()
        .map(|group| {
            group
                .as_array()
                .unwrap_or_else(|| panic!("{} field {key} must contain arrays", path.display()))
                .iter()
                .map(|item| {
                    item.as_str()
                        .unwrap_or_else(|| {
                            panic!("{} field {key} groups must contain strings", path.display())
                        })
                        .to_string()
                })
                .collect()
        })
        .collect()
}
