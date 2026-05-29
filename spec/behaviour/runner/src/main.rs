mod capture;
mod checks;
mod execution;
mod input;
mod json;
mod runtime;
mod seed;
mod validation;
mod vocabulary;
mod world;

use capture::CapturedOutbound;
use checks::OutputChecks;
use clap::{Args, Parser, Subcommand};
use execution::{CaseExecution, ExecutionOptions};
use input::CaseInput;
use json::{required_array, required_object, required_str};
use serde_json::Value;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub struct BehaviourCase {
    pub path: PathBuf,
    pub value: Value,
}

#[derive(Parser)]
#[command(name = "behaviour-runner")]
#[command(about = "Validate and execute Pamagotchi behaviour specs")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Validate,
    Execute(ExecuteArgs),
}

#[derive(Args)]
struct ExecuteArgs {
    #[arg(long)]
    case: Option<String>,
    #[arg(long)]
    tag: Option<String>,
    #[arg(long)]
    priority: Option<String>,
    #[arg(long)]
    repeat: Option<usize>,
    #[arg(long)]
    no_stream: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let root = repo_root();

    match cli.command {
        Command::Validate => validate(&root),
        Command::Execute(args) => execute(&root, &args).await,
    }
}

fn validate(root: &Path) -> anyhow::Result<()> {
    let cases = validation::load_validated_cases(root);
    println!("behaviour spec validation");
    println!("cases: {}", cases.len());
    println!("status: ok");
    Ok(())
}

async fn execute(root: &Path, args: &ExecuteArgs) -> anyhow::Result<()> {
    let runtime =
        runtime::RuntimeConfig::load(root).expect("failed to load behaviour runtime config");
    let cases = validation::load_validated_cases(root);
    let selected = select_cases(&cases, args);
    let repeat = repeat_count(args.repeat);

    if selected.is_empty() {
        anyhow::bail!("no behaviour cases matched --case/--tag/--priority or env filters");
    }

    println!("behaviour spec execution");
    println!("selected_cases: {}", selected.len());
    println!("repeat: {repeat}");
    println!("runtime: {}", runtime.summary());
    println!(
        "runtime_routes: chat_supports_text={}, chat_supports_vision={}",
        runtime.router.chat_supports(&[]),
        runtime
            .router
            .chat_supports(&[inference::Capability::Vision])
    );
    println!("actor_execution: live");
    println!();
    flush_stdout();

    let mut failed = 0usize;

    for case in selected {
        let mut case_failed = false;
        let mut run_outputs = Vec::new();

        for run_idx in 0..repeat {
            if repeat > 1 {
                println!("run: {}/{}", run_idx + 1, repeat);
            }
            let world = world::seed_world(case)
                .await
                .unwrap_or_else(|err| panic!("failed to seed {}: {err}", case.path.display()));
            let counts = world.counts.clone();
            let input = input::build_case_input(case, &world.contexts).unwrap_or_else(|err| {
                panic!("failed to build input for {}: {err}", case.path.display())
            });
            print_case_start(case, &counts, &input);

            let execution = execution::execute_case_with_input(
                &runtime,
                world,
                input,
                ExecutionOptions {
                    stream_output: !args.no_stream,
                },
            )
            .await
            .unwrap_or_else(|err| panic!("failed to execute {}: {err}", case.path.display()));
            let checks =
                checks::evaluate_output(&runtime, case, &execution.output, execution.timed_out)
                    .await;
            let state_check = checks::evaluate_state(case, &execution);
            print_case_finish(&execution, &checks, &state_check, !args.no_stream);
            if !checks.passed() || !state_check.passed {
                case_failed = true;
            }
            run_outputs.push(execution.output);
            if repeat > 1 {
                if checks.passed() && state_check.passed {
                    println!("run_result: pass");
                } else {
                    println!("run_result: fail");
                }
                println!();
            }
            flush_stdout();
        }

        let repeat_check = checks::evaluate_repeated_outputs(case, &run_outputs);
        if repeat > 1 || !repeat_check.passed {
            println!("aggregate_checks:");
            print_check("repeat_freshness", &repeat_check);
        }
        if !repeat_check.passed {
            case_failed = true;
        }

        if case_failed {
            failed += 1;
            println!("case_result: fail");
        } else {
            println!("case_result: pass");
        }
        println!();
        flush_stdout();
    }

    if failed > 0 {
        anyhow::bail!("{failed} behaviour case(s) failed");
    }
    Ok(())
}

fn select_cases<'a>(cases: &'a [BehaviourCase], args: &ExecuteArgs) -> Vec<&'a BehaviourCase> {
    let case_filter = arg_or_env(args.case.as_deref(), "BEHAVIOUR_CASE");
    let tag_filter = arg_or_env(args.tag.as_deref(), "BEHAVIOUR_TAG");
    let priority_filter = arg_or_env(args.priority.as_deref(), "BEHAVIOUR_PRIORITY");

    cases
        .iter()
        .filter(|case| {
            matches_case_filter(case, case_filter.as_deref())
                && matches_tag_filter(case, tag_filter.as_deref())
                && matches_priority_filter(case, priority_filter.as_deref())
        })
        .collect()
}

fn arg_or_env(arg: Option<&str>, env_key: &str) -> Option<String> {
    arg.map(str::to_string)
        .or_else(|| std::env::var(env_key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn repeat_count(arg: Option<usize>) -> usize {
    arg.or_else(|| {
        std::env::var("BEHAVIOUR_REPEAT")
            .ok()
            .and_then(|value| value.parse().ok())
    })
    .unwrap_or(1)
    .max(1)
}

fn matches_case_filter(case: &BehaviourCase, filter: Option<&str>) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    required_str(&case.value, "id", &case.path) == filter
}

fn matches_tag_filter(case: &BehaviourCase, filter: Option<&str>) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    required_array(&case.value, "tags", &case.path)
        .iter()
        .any(|tag| tag.as_str() == Some(filter))
}

fn matches_priority_filter(case: &BehaviourCase, filter: Option<&str>) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    required_str(&case.value, "priority", &case.path) == filter
}

fn print_case_start(case: &BehaviourCase, counts: &world::SeedCounts, input: &CaseInput) {
    let id = required_str(&case.value, "id", &case.path);
    let title = required_str(&case.value, "title", &case.path);
    let priority = required_str(&case.value, "priority", &case.path);

    println!("{id} {title}");
    println!("priority: {priority}");
    print_tags(case);
    print_scenario(case);
    print_seed_counts(counts);
    print_input(input);
    println!();
    println!("output:");
    flush_stdout();
}

fn print_case_finish(
    execution: &CaseExecution,
    checks: &OutputChecks,
    state_check: &checks::CheckOutcome,
    output_already_streamed: bool,
) {
    if !output_already_streamed {
        print_captured_output(&execution.output);
    } else if execution.output.is_empty() {
        println!("  <none>");
    }
    if execution.timed_out {
        println!("  status: timed out waiting for expected output");
    }
    println!();
    println!("checks:");
    print_check("cadence", &checks.cadence);
    print_check("freshness", &checks.freshness);
    print_check("required_beats", &checks.required_beats);
    print_check("forbidden_beats", &checks.forbidden_beats);
    print_check("state", state_check);
    flush_stdout();
}

fn print_captured_output(messages: &[CapturedOutbound]) {
    if messages.is_empty() {
        println!("  <none>");
        return;
    }

    for (idx, message) in messages.iter().enumerate() {
        let suffix = if message.attachment_count > 0 {
            format!(" attachments={}", message.attachment_count)
        } else {
            String::new()
        };
        println!(
            "  actor[{}]({}/{}{}): {}",
            idx + 1,
            message.gateway_id,
            message.external_id,
            suffix,
            message.content
        );
    }
}

fn print_seed_counts(counts: &world::SeedCounts) {
    println!();
    println!("seed:");
    println!(
        "  people: {}, profiles: {}, identities: {}, groups: {}, memories: {}, conversations: {}, conversation_messages: {}, pending_identity_claims: {}",
        counts.people,
        counts.profiles,
        counts.identities,
        counts.groups,
        counts.memories,
        counts.conversations,
        counts.conversation_messages,
        counts.pending_identity_claims,
    );
}

fn print_tags(case: &BehaviourCase) {
    println!("tags: {}", join_string_array(&case.value, "tags", case));
}

fn print_scenario(case: &BehaviourCase) {
    let scenario = required_object(&case.value, "scenario", &case.path);
    println!();
    println!("scenario:");
    println!("  who: {}", required_str(scenario, "who", &case.path));
    println!("  when: {}", required_str(scenario, "when", &case.path));
    println!(
        "  what_happened: {}",
        required_str(scenario, "what_happened", &case.path)
    );
}

fn print_input(input: &CaseInput) {
    println!();
    println!("input:");
    for message in &input.messages {
        let profile = message
            .profile
            .as_ref()
            .map(|profile| profile.0.as_str())
            .unwrap_or("unseeded");
        println!(
            "  user({}/{}, profile={}): {}",
            message.gateway_id, message.sender_external_id, profile, message.content
        );
    }
}

fn print_check(name: &str, outcome: &checks::CheckOutcome) {
    let status = if outcome.passed { "pass" } else { "fail" };
    println!("  {name}: {status} ({})", outcome.detail);
}

fn join_string_array(value: &Value, key: &str, case: &BehaviourCase) -> String {
    required_array(value, key, &case.path)
        .iter()
        .map(|item| {
            item.as_str().unwrap_or_else(|| {
                panic!("{} field {key} must contain strings", case.path.display())
            })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn repo_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .find(|path| path.join("spec/runtime.yaml").is_file())
        .unwrap_or_else(|| panic!("failed to find repo root from {}", manifest_dir.display()))
        .to_path_buf()
}

pub fn case_paths(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()))
        .map(|entry| {
            entry
                .expect("failed to read behaviour case dir entry")
                .path()
        })
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect()
}

pub fn load_yaml(path: &Path) -> Value {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    yaml_serde::from_str(&raw)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
}

fn flush_stdout() {
    let _ = io::stdout().flush();
}
