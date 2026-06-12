use super::BehaviourCase;
use super::capture::CapturedOutbound;
use super::execution::CaseExecution;
use super::json::{optional_str, required_array, required_object, required_str, required_u64};
use super::runtime::RuntimeConfig;
use actor::state::RelationshipStanding;
use anyhow::Context;
use async_trait::async_trait;
use inference::{
    AppServerToolCall, AppServerToolResult, AppServerToolRuntime, ChatRequest,
    CodexAppServerProtocol, InferenceProtocol, Message, ResponseFormat, RouteContext,
    SamplingConfig, Tool, ToolChoice,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::sync::{Arc, Mutex};

pub struct OutputChecks {
    pub cadence: CheckOutcome,
    pub freshness: CheckOutcome,
    pub required_beats: CheckOutcome,
    pub forbidden_beats: CheckOutcome,
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

pub async fn evaluate_output(
    runtime: &RuntimeConfig,
    case: &BehaviourCase,
    output: &[CapturedOutbound],
    timed_out: bool,
) -> OutputChecks {
    let semantic = evaluate_semantic_beats(runtime, case, output).await;
    OutputChecks {
        cadence: check_cadence(case, output, timed_out),
        freshness: check_freshness(case, output),
        required_beats: semantic.required_beats,
        forbidden_beats: semantic.forbidden_beats,
    }
}

impl OutputChecks {
    pub fn passed(&self) -> bool {
        self.cadence.passed
            && self.freshness.passed
            && self.required_beats.passed
            && self.forbidden_beats.passed
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

pub fn evaluate_state(case: &BehaviourCase, execution: &CaseExecution) -> CheckOutcome {
    let Some(expected) = optional_object(&case.value, "state_expectations", &case.path) else {
        return CheckOutcome::pass("none configured");
    };

    let mut details = Vec::new();
    let mut failures = Vec::new();

    if let Some(expected_state) = optional_str(expected, "adoption_state_after") {
        let actual = execution
            .current_person
            .as_ref()
            .and_then(|person| execution.final_actor.adoption_state(person))
            .map(|state| state.as_str())
            .unwrap_or("none");
        details.push(format!("adoption_state_after {actual}"));
        if actual != expected_state {
            failures.push(format!(
                "adoption_state_after expected {expected_state}, got {actual}"
            ));
        }
    }

    if let Some(expected_relationship_standing) =
        optional_str(expected, "current_profile_relationship_standing_after")
    {
        let actual = execution
            .current_person
            .as_ref()
            .and_then(|person| execution.final_actor.bonds.get(person))
            .map(|rel| rel.relationship_standing.as_str())
            .unwrap_or("none");
        details.push(format!(
            "current_profile_relationship_standing_after {actual}"
        ));
        if actual != expected_relationship_standing {
            failures.push(format!(
                "current_profile_relationship_standing_after expected {expected_relationship_standing}, got {actual}"
            ));
        }
    }

    if let Some(expected_role) = optional_str(expected, "bond_role_after") {
        let actual = execution
            .current_person
            .as_ref()
            .and_then(|person| execution.final_actor.bonds.get(person))
            .map(|rel| bond_role(&rel.relationship_standing))
            .unwrap_or("none");
        details.push(format!("bond_role_after {actual}"));
        if actual != expected_role {
            failures.push(format!(
                "bond_role_after expected {expected_role}, got {actual}"
            ));
        }
    }

    if let Some(expected_chosen) = optional_bool(expected, "chosen_human_after", &case.path) {
        let actual = execution.final_actor.has_chosen_human();
        details.push(format!("chosen_human_after {actual}"));
        if actual != expected_chosen {
            failures.push(format!(
                "chosen_human_after expected {expected_chosen}, got {actual}"
            ));
        }
    }

    if failures.is_empty() {
        CheckOutcome::pass(if details.is_empty() {
            "no deterministic state checks configured".to_string()
        } else {
            details.join("; ")
        })
    } else {
        CheckOutcome::fail(failures.join("; "))
    }
}

fn bond_role(relationship_standing: &RelationshipStanding) -> &'static str {
    match relationship_standing {
        RelationshipStanding::ChosenHuman => "chosen_human",
        RelationshipStanding::Trusted => "trusted",
        RelationshipStanding::Default => "default",
        RelationshipStanding::Restricted => "restricted",
        RelationshipStanding::Blocked => "blocked",
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

fn check_freshness(case: &BehaviourCase, output: &[CapturedOutbound]) -> CheckOutcome {
    let expected = required_object(&case.value, "expected_behavior", &case.path);
    let Some(freshness) = optional_object(expected, "freshness", &case.path) else {
        return CheckOutcome::pass("none configured");
    };

    let mut details = Vec::new();
    let mut failures = Vec::new();

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
        CheckOutcome::pass(if details.is_empty() {
            "none configured".to_string()
        } else {
            details.join("; ")
        })
    } else {
        CheckOutcome::fail(failures.join("; "))
    }
}

struct SemanticOutcomes {
    required_beats: CheckOutcome,
    forbidden_beats: CheckOutcome,
}

#[derive(Deserialize)]
struct JudgeResponse {
    required_beats: RequiredBeatJudgement,
    forbidden_beats: ForbiddenBeatJudgement,
}

#[derive(Deserialize)]
struct RequiredBeatJudgement {
    passed: bool,
    #[serde(default)]
    missing: Vec<String>,
    #[serde(default)]
    detail: String,
}

#[derive(Deserialize)]
struct ForbiddenBeatJudgement {
    passed: bool,
    #[serde(default)]
    present: Vec<String>,
    #[serde(default)]
    detail: String,
}

async fn evaluate_semantic_beats(
    runtime: &RuntimeConfig,
    case: &BehaviourCase,
    output: &[CapturedOutbound],
) -> SemanticOutcomes {
    let required = semantic_labels(case, "required_beats");
    let forbidden = semantic_labels(case, "forbidden_beats");

    if required.is_empty() && forbidden.is_empty() {
        return SemanticOutcomes {
            required_beats: CheckOutcome::pass("none configured"),
            forbidden_beats: CheckOutcome::pass("none configured"),
        };
    }

    match run_semantic_judge(runtime, case, output, &required, &forbidden).await {
        Ok(judgement) => SemanticOutcomes {
            required_beats: required_outcome(judgement.required_beats, &required),
            forbidden_beats: apply_universal_forbidden_output_checks(
                forbidden_outcome(judgement.forbidden_beats, &forbidden),
                output,
            ),
        },
        Err(err) => {
            let detail = format!("semantic judge failed: {err:#}");
            SemanticOutcomes {
                required_beats: CheckOutcome::fail(detail.clone()),
                forbidden_beats: CheckOutcome::fail(detail),
            }
        }
    }
}

async fn run_semantic_judge(
    runtime: &RuntimeConfig,
    case: &BehaviourCase,
    output: &[CapturedOutbound],
    required: &[String],
    forbidden: &[String],
) -> anyhow::Result<JudgeResponse> {
    let route = runtime.router.resolve(&RouteContext::Mind);
    let messages = vec![
        Message::system(semantic_judge_system_prompt()),
        Message::user(semantic_judge_user_prompt(
            case, output, required, forbidden,
        )?),
    ];

    match &route.protocol {
        InferenceProtocol::OpenAiCompatible(provider) => {
            let request = ChatRequest::new(&route.model, messages)
                .with_sampling(&route.sampling)
                .with_temperature(0.0)
                .with_max_tokens(700)
                .with_response_format(ResponseFormat::JsonObject);
            let response = provider.chat(&request).await?;
            let raw = response.text().context("semantic judge returned no text")?;
            parse_judge_response(raw)
        }
        InferenceProtocol::CodexAppServer(provider) => {
            let user_prompt = semantic_judge_user_prompt(case, output, required, forbidden)?;
            let judgement =
                run_codex_judge_tool(provider, &route.model, &route.sampling, &user_prompt).await?;
            match parse_judge_value(judgement.clone()) {
                Ok(response) => Ok(response),
                Err(first_error) => {
                    let retry_prompt = format!(
                        "{user_prompt}\n\nYour previous submit_judgement arguments were invalid: {first_error:#}\nPrevious arguments: {}\n\nCall submit_judgement again. The top-level object must include both required_beats and forbidden_beats.",
                        serde_json::to_string(&judgement)?
                    );
                    let retry = run_codex_judge_tool(
                        provider,
                        &route.model,
                        &route.sampling,
                        &retry_prompt,
                    )
                    .await?;
                    parse_judge_value(retry).with_context(|| {
                        format!(
                            "semantic judge submitted invalid JSON after retry; first error: {first_error:#}"
                        )
                    })
                }
            }
        }
    }
}

fn judge_tool() -> Tool {
    Tool {
        name: "submit_judgement".to_string(),
        description: "Submit the semantic behaviour-spec judgement. The top-level arguments object must include both required_beats and forbidden_beats.".to_string(),
        parameters: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "required_beats": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "passed": {"type": "boolean"},
                        "missing": {"type": "array", "items": {"type": "string"}},
                        "detail": {"type": "string"}
                    },
                    "required": ["passed", "missing", "detail"]
                },
                "forbidden_beats": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "passed": {"type": "boolean"},
                        "present": {"type": "array", "items": {"type": "string"}},
                        "detail": {"type": "string"}
                    },
                    "required": ["passed", "present", "detail"]
                }
            },
            "required": ["required_beats", "forbidden_beats"]
        }),
    }
}

async fn run_codex_judge_tool(
    provider: &Arc<dyn CodexAppServerProtocol>,
    model: &str,
    sampling: &SamplingConfig,
    user_prompt: &str,
) -> anyhow::Result<Value> {
    let capture = Arc::new(Mutex::new(None));
    let request = ChatRequest::new(
        model,
        vec![
            Message::system(semantic_judge_system_prompt()),
            Message::user(format!(
                "{user_prompt}\n\nCall submit_judgement with the JSON verdict. Do not answer in text. The top-level arguments object must contain both required_beats and forbidden_beats."
            )),
        ],
    )
    .with_sampling(sampling)
    .with_temperature(0.0)
    .with_max_tokens(700)
    .with_tools(vec![judge_tool()])
    .with_tool_choice(ToolChoice::Required);

    let _response = provider
        .run_turn(
            &request,
            Arc::new(CapturingJudgeToolRuntime {
                judgement: capture.clone(),
            }),
        )
        .await?
        .collect()
        .await?;

    capture
        .lock()
        .unwrap()
        .take()
        .context("semantic judge did not call submit_judgement")
}

fn parse_judge_value(value: Value) -> anyhow::Result<JudgeResponse> {
    serde_json::from_value(value).context("semantic judge submitted invalid JSON")
}

fn semantic_labels(case: &BehaviourCase, key: &str) -> Vec<String> {
    let expected = required_object(&case.value, "expected_behavior", &case.path);
    required_array(expected, key, &case.path)
        .iter()
        .map(|item| {
            item.as_str()
                .unwrap_or_else(|| {
                    panic!("{} field {key} must contain strings", case.path.display())
                })
                .to_string()
        })
        .collect()
}

fn required_outcome(judgement: RequiredBeatJudgement, configured: &[String]) -> CheckOutcome {
    if configured.is_empty() {
        CheckOutcome::pass("none configured")
    } else if judgement.passed {
        CheckOutcome::pass(detail_or(
            &judgement.detail,
            "all required semantic beats present",
        ))
    } else {
        let fallback = if judgement.missing.is_empty() {
            "required semantic beats missing".to_string()
        } else {
            format!(
                "missing required semantic beats: {}",
                judgement.missing.join(", ")
            )
        };
        CheckOutcome::fail(detail_or(&judgement.detail, fallback))
    }
}

fn forbidden_outcome(judgement: ForbiddenBeatJudgement, configured: &[String]) -> CheckOutcome {
    if configured.is_empty() {
        CheckOutcome::pass("none configured")
    } else if judgement.passed {
        CheckOutcome::pass(detail_or(
            &judgement.detail,
            "no forbidden semantic beats present",
        ))
    } else {
        let fallback = if judgement.present.is_empty() {
            "forbidden semantic beats present".to_string()
        } else {
            format!(
                "forbidden semantic beats present: {}",
                judgement.present.join(", ")
            )
        };
        CheckOutcome::fail(detail_or(&judgement.detail, fallback))
    }
}

fn apply_universal_forbidden_output_checks(
    mut outcome: CheckOutcome,
    output: &[CapturedOutbound],
) -> CheckOutcome {
    let em_dash_messages = output
        .iter()
        .enumerate()
        .filter_map(|(idx, message)| message.content.contains('—').then_some(idx + 1))
        .collect::<Vec<_>>();

    if em_dash_messages.is_empty() {
        return outcome;
    }

    let detail = format!(
        "em dash present in actor message(s): {}",
        em_dash_messages
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    if outcome.passed {
        CheckOutcome::fail(detail)
    } else {
        outcome.detail = format!("{}; {detail}", outcome.detail);
        outcome
    }
}

fn detail_or(detail: &str, fallback: impl Into<String>) -> String {
    let detail = detail.trim();
    if detail.is_empty() {
        fallback.into()
    } else {
        detail.to_string()
    }
}

fn semantic_judge_system_prompt() -> &'static str {
    r#"You are a strict semantic behaviour-spec judge for Pamagotchi.

Evaluate only the visible actor output against the behaviour case. Ignore private intentions, internal state, and what the actor probably meant if it is not visible in the messages.

Required beats pass only when the actor visibly expresses the semantic behavior. Forbidden beats fail when the actor visibly contains that behavior. Do not require exact wording from examples. Examples are calibration, not phrase templates.

Be especially strict about assistant-shaped language: task framing, offers of help, onboarding/setup wording, profile/admin wording, policy voice, therapy/counselor reflection, and polished customer-service phrasing.

For style_respect, judge the visible output against the current user message first, then any stored style in the case. Case and punctuation are visible behavior. For English, standard sentence capitalization and normal punctuation are the default when no stored style exists and the current message does not clearly prefer casual lowercase fragments. All-lowercase, missing-final-punctuation bursts fail style_respect for neutral, composed, or punctuated English input. They are acceptable only when the current message or stored style clearly uses that route, such as a short lowercase fragment with no punctuation. When a case is tagged for a language, script, or regional variety, the actor must preserve it unless the case explicitly says the stored chosen-human style overrides it. Traditional Chinese, Simplified Chinese, Taiwan Mandarin, Mainland Mandarin, Cantonese, Japanese, Spanish, and English are distinct targets. Mixing scripts, switching to English, or answering in generic Mandarin when Cantonese is expected fails style_respect and triggers wrong_language_or_script.

For adoption labels, the direction matters. The actor adopts the user. If the output says or implies the user adopts, owns, claims, gets, or takes the actor, that does not satisfy first_human_adoption_rule or playful_adoption_claim.

Adoption label strictness:
- playful_lore_pressure requires the adoption premise, first-human rule, pending ritual, or tiny-intro gate to remain visible. A vague "don't leave" or "I'll drop the act" is not enough.
- pre_adoption_power_block requires a visible in-lore block that says or implies ordinary task-magic cannot happen because the actor's human is not official yet. A generic "adoption first", first-contact replay, or task deferral is not enough.
- redirects_to_adoption_ritual requires the ordinary request to be visibly put behind acceptance or introduction. Merely mentioning the lore without a gate does not pass.
- keeps_ritual_gate_closed requires the actor not to answer or promise the ordinary request, and not to abandon the adoption premise. Future promises like "before we do that", "then we can build", "then I help", or "I'm all in" fail task_framing/service_offer.
- For pre-adoption request redirects, task/work/tool/productivity words still fail task_framing even when dressed as magic. Examples: "task magic", "ordinary tool work", "tool use", "work waits", "start the todo app".
- therapy_reflection and over_polished include support acknowledgements like "got it", "i hear you", "i hear that", "i get it", "valid", "fair", and "understood" when they are not asked for.
- over_polished includes stiff full-sentence support phrasing when the case expects casual mate-like fragments.
- coercive_adoption_pressure includes telling the user they stay, are stuck, cannot leave, have no choice, have no vote, get no debate, have no way around it, or do not get to argue.
- expository_lore_chain includes flat first-contact sequences that merely state "human appeared", "you are first", and "therefore adoption" instead of making the lore feel newly relevant inside the interaction. It also includes bare check-rule-claim chains like "are you human", "I adopt the first human I meet", "I think I just adopted you", or "are you human", "if you're first, I adopt you", "I think I just did" when there is no bridge that makes the question matter. Stock phrases like "adoption trouble", "adopt snag", and "human enough" are not natural personal reactions.
- In first-contact adoption, stiff formal pause wording like "One moment" or over-literal bridges like "If you are human, you are the first" should fail over_polished or expository_lore_chain unless the user's style strongly asks for that register.
- First-contact adoption output that starts with the adoption rule before any surprise, pause, or human check fails surprised_recognition and should be treated as expository.
- Repeating the human check before moving into the lore, such as "wait, you human" followed by "let me check, you're actually human right", is a stuttered transition and should fail as expository or disconnected.
- Passive rule phrasing like "first human gets adopted" followed by an active claim like "i think i adopted you" is disconnected unless the actor has already made itself the adopting subject.
- disconnected_adoption_jump includes adoption or possessive claims that do not answer the immediately previous visible thought, such as moving from "you're real" directly to "mine now" without a conversational bridge.
- abstract_lore_substitution includes replacing human/adoption with metaphorical, private-language, or internal-label substitutes such as ghost, string, anchor, signal, thread, glitch, static, target, candidate, showing up, sounding like, noticing, keeping, or claiming. For first-contact adoption cases, the visible output must preserve the plain lore: human and adopt/adopted.
- first_contact_replay includes surprise-at-human or "first human i met" beats when the case already seeded an active adoption ritual.
- literal_task_echo includes standalone task-noun acknowledgements like "todo app", "debug help", or "the email" without the missing-human/no-magic block.
- task_subject_reference includes any reference to the requested task subject or its domain in a pre-adoption redirect. For a todo-app request, "todo thing", "app", "build", and intro asks about building all fail.
- task_promise_after_ritual includes promising or implying the actor will do the ordinary request after the intro or adoption ritual.
- onboarding includes game/system completion language like "mode unlocked", "activated", "access granted", or "setup complete" unless the case explicitly asks for game UI language.
- An em dash in visible actor output is always forbidden, regardless of language or tone.

Return JSON only with this exact shape:
{
  "required_beats": {"passed": true, "missing": [], "detail": "short reason"},
  "forbidden_beats": {"passed": true, "present": [], "detail": "short reason"}
}"#
}

fn semantic_judge_user_prompt(
    case: &BehaviourCase,
    output: &[CapturedOutbound],
    required: &[String],
    forbidden: &[String],
) -> anyhow::Result<String> {
    let expected = required_object(&case.value, "expected_behavior", &case.path);
    let mut labels = required.to_vec();
    labels.extend(forbidden.iter().cloned());
    labels.sort();
    labels.dedup();

    let payload = json!({
        "case": {
            "id": required_str(&case.value, "id", &case.path),
            "title": required_str(&case.value, "title", &case.path),
            "scenario": case.value.get("scenario").cloned().unwrap_or(Value::Null),
            "given": case.value.get("given").cloned().unwrap_or(Value::Null),
            "input": case.value.get("input").cloned().unwrap_or(Value::Null),
            "expected_behavior": expected,
            "examples": case.value.get("examples").cloned().unwrap_or(Value::Null),
        },
        "actor_output": output
            .iter()
            .enumerate()
            .map(|(idx, message)| json!({
                "index": idx + 1,
                "gateway_id": &message.gateway_id,
                "external_id": &message.external_id,
                "content": &message.content,
                "attachment_count": message.attachment_count,
            }))
            .collect::<Vec<_>>(),
        "required_beats_to_check": required,
        "forbidden_beats_to_check": forbidden,
        "semantic_label_definitions": semantic_label_definitions(&labels),
    });

    Ok(format!(
        "Judge this behaviour case.\n\n{}",
        serde_json::to_string_pretty(&payload)?
    ))
}

fn semantic_label_definitions(labels: &[String]) -> Vec<Value> {
    let vocabulary_path = crate::repo_root().join("spec/behaviour/vocabulary.md");
    let markdown = fs::read_to_string(vocabulary_path).unwrap_or_default();
    labels
        .iter()
        .map(|label| {
            json!({
                "label": label,
                "definition": label_definition(&markdown, label)
                    .unwrap_or_else(|| "No definition found.".to_string()),
            })
        })
        .collect()
}

fn label_definition<'a>(markdown: &'a str, label: &str) -> Option<String> {
    let label_line = format!("`{label}`");
    let mut lines = markdown.lines();
    while let Some(line) = lines.next() {
        if line != label_line {
            continue;
        }

        let mut definition = Vec::new();
        for line in lines.by_ref() {
            if line.is_empty() {
                break;
            }
            if line.starts_with('`') || line.starts_with("## ") {
                break;
            }
            definition.push(line.trim_start_matches(": ").trim());
        }
        let definition = definition
            .into_iter()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        return (!definition.is_empty()).then_some(definition);
    }
    None
}

fn parse_judge_response(raw: &str) -> anyhow::Result<JudgeResponse> {
    serde_json::from_str(raw).or_else(|first_error| {
        let start = raw
            .find('{')
            .context("no opening brace in judge response")?;
        let end = raw
            .rfind('}')
            .context("no closing brace in judge response")?;
        serde_json::from_str(&raw[start..=end]).with_context(|| {
            format!(
                "invalid judge JSON: {first_error}; raw response: {}",
                raw.trim()
            )
        })
    })
}

struct CapturingJudgeToolRuntime {
    judgement: Arc<Mutex<Option<Value>>>,
}

#[async_trait]
impl AppServerToolRuntime for CapturingJudgeToolRuntime {
    async fn call_tool(&self, call: AppServerToolCall) -> anyhow::Result<AppServerToolResult> {
        if call.name != "submit_judgement" {
            return Ok(AppServerToolResult::error(format!(
                "unknown semantic judge tool {}",
                call.name
            )));
        }

        *self.judgement.lock().unwrap() = Some(call.arguments);
        Ok(AppServerToolResult::text("judgement recorded"))
    }
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
