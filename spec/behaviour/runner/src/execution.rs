use super::capture::{CaptureSink, CapturedOutbound, RecordingGateway};
use super::input::CaseInput;
use super::runtime::RuntimeConfig;
use super::world::SeededWorld;
use actor::core::{Actor, ActorLifecycleEvent, WakeEvent};
use actor::store::Store;
use gateway::GatewayRouter;
use protocol::InboundMessage;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep};

pub struct CaseExecution {
    pub output: Vec<CapturedOutbound>,
    pub timed_out: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ExecutionOptions {
    pub stream_output: bool,
}

pub async fn execute_case_with_input(
    runtime: &RuntimeConfig,
    world: SeededWorld,
    input: CaseInput,
    options: ExecutionOptions,
) -> anyhow::Result<CaseExecution> {
    let sink = CaptureSink::default();
    let gateway = recording_gateway(&input.gateway_ids, &sink);
    let store = Arc::new(world.store);
    let actor_store: Arc<dyn Store> = store.clone();
    let source_message_keys = source_message_keys(&input.messages);
    let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
    let actor = Actor::builder(actor_store, runtime.router.clone())
        .with_state(world.actor)
        .with_gateway(gateway)
        .with_lifecycle_events(lifecycle_tx)
        .with_max_concurrency(1)
        .with_max_turns(max_turns())
        .with_retry(max_action_attempts(), 1)
        .build()
        .await?;

    for message in &input.messages {
        actor
            .send_event(WakeEvent::Message(message.clone()))
            .await?;
    }

    let wait = wait_for_response_completion(
        &sink,
        lifecycle_rx,
        &source_message_keys,
        execution_timeout(),
        options.stream_output,
    )
    .await;

    actor.shutdown().await?;

    Ok(CaseExecution {
        output: wait.messages,
        timed_out: wait.timed_out,
    })
}

struct OutputWait {
    messages: Vec<CapturedOutbound>,
    timed_out: bool,
}

fn recording_gateway(gateway_ids: &[String], sink: &CaptureSink) -> Arc<GatewayRouter> {
    let gateway = Arc::new(GatewayRouter::new());
    for gateway_id in gateway_ids {
        gateway.register(Arc::new(RecordingGateway::new(
            gateway_id.clone(),
            sink.clone(),
        )));
    }
    gateway
}

async fn wait_for_response_completion(
    sink: &CaptureSink,
    mut lifecycle_rx: mpsc::UnboundedReceiver<ActorLifecycleEvent>,
    source_message_keys: &[String],
    timeout: Duration,
    stream_output: bool,
) -> OutputWait {
    let deadline = Instant::now() + timeout;
    let mut streamed_len = 0;

    loop {
        streamed_len = maybe_print_streamed_output(sink, streamed_len, stream_output);

        let now = Instant::now();
        if now >= deadline {
            return OutputWait {
                messages: sink.messages(),
                timed_out: true,
            };
        }

        let remaining_deadline = deadline.saturating_duration_since(now);

        tokio::select! {
            event = lifecycle_rx.recv() => {
                let Some(event) = event else {
                    return OutputWait {
                        messages: sink.messages(),
                        timed_out: true,
                    };
                };
                if matches_response_completion(&event, source_message_keys) {
                    maybe_print_streamed_output(sink, streamed_len, stream_output);
                    return OutputWait {
                        messages: sink.messages(),
                        timed_out: false,
                    };
                }
            }
            _ = sink.wait_for_change() => {}
            _ = sleep(remaining_deadline) => {
                return OutputWait {
                    messages: sink.messages(),
                    timed_out: true,
                };
            }
        }
    }
}

fn matches_response_completion(
    event: &ActorLifecycleEvent,
    source_message_keys: &[String],
) -> bool {
    let ActorLifecycleEvent::ActionCompleted(completed) = event else {
        return false;
    };
    completed.kind.as_str() == "respond"
        && completed
            .source_message_keys
            .iter()
            .any(|key| source_message_keys.contains(key))
}

fn maybe_print_streamed_output(
    sink: &CaptureSink,
    streamed_len: usize,
    stream_output: bool,
) -> usize {
    if !stream_output {
        return streamed_len;
    }
    let messages = sink.messages();
    if messages.len() > streamed_len {
        print_streamed_output(&messages, streamed_len);
        messages.len()
    } else {
        streamed_len
    }
}

fn source_message_keys(messages: &[InboundMessage]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| {
            if message.gateway_id.is_empty() || message.message_id.is_empty() {
                None
            } else {
                Some(format!("{}:{}", message.gateway_id, message.message_id))
            }
        })
        .collect()
}

fn print_streamed_output(messages: &[CapturedOutbound], start: usize) {
    for (idx, message) in messages.iter().enumerate().skip(start) {
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
    flush_stdout();
}

fn flush_stdout() {
    let _ = io::stdout().flush();
}

fn execution_timeout() -> Duration {
    env_duration_secs("BEHAVIOUR_TIMEOUT_SECS", 120)
}

fn max_turns() -> usize {
    env_usize("BEHAVIOUR_MAX_TURNS", 5)
}

fn max_action_attempts() -> usize {
    env_usize("BEHAVIOUR_MAX_ACTION_ATTEMPTS", 2)
}

fn env_duration_secs(name: &str, default: u64) -> Duration {
    Duration::from_secs(env_u64(name, default))
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
