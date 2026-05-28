use super::BehaviourCase;
use super::capture::CapturedOutbound;
use super::json::{required_array, required_object, required_str, required_u64};

pub struct OutputChecks {
    pub cadence: CheckOutcome,
    pub forbidden_phrases: CheckOutcome,
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
    }
}

impl OutputChecks {
    pub fn passed(&self) -> bool {
        self.cadence.passed && self.forbidden_phrases.passed
    }
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
    let combined = output
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    let hits = required_array(expected, "forbidden_phrases", &case.path)
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
