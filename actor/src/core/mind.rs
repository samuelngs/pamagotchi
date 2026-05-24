use super::action::{
    ActionBrief, ActionContext, ActionId, ActionKind, ActionProgress, ActionRequest, ActionState,
    ActionStatus, ActionTiming, MindDecision,
};
use super::event::{InboundMessage, WakeEvent};
use super::registry::ActionRegistry;
use super::state::StateHandle;
use super::tools::mind_tools;
use crate::llm::{ChatRequest, Message, Provider, SamplingConfig, ToolChoice};
use crate::personality::Authority;
use crate::platform::PlatformRouter;
use crate::store::{MessageRole, Store};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

pub struct Mind {
    event_rx: mpsc::Receiver<WakeEvent>,
    event_tx: mpsc::Sender<WakeEvent>,
    registry: ActionRegistry,
    state: StateHandle,
    store: Arc<dyn Store>,
    provider: Arc<dyn Provider>,
    platform: Arc<PlatformRouter>,
    model: String,
    sampling: SamplingConfig,
}

impl Mind {
    pub fn new(
        event_rx: mpsc::Receiver<WakeEvent>,
        event_tx: mpsc::Sender<WakeEvent>,
        state: StateHandle,
        store: Arc<dyn Store>,
        provider: Arc<dyn Provider>,
        platform: Arc<PlatformRouter>,
        model: String,
        sampling: SamplingConfig,
        max_concurrency: usize,
    ) -> Self {
        Self {
            event_rx,
            event_tx,
            registry: ActionRegistry::new(max_concurrency),
            state,
            store,
            provider,
            platform,
            model,
            sampling,
        }
    }

    pub async fn run(mut self) {
        info!("mind started");
        loop {
            match self.event_rx.recv().await {
                Some(WakeEvent::Shutdown) => {
                    self.shutdown().await;
                    break;
                }
                Some(WakeEvent::ActionCompleted { action_id, result }) => {
                    self.registry.mark_completed(&action_id);
                    self.handle_action_completed(&action_id, &result).await;
                    let event = WakeEvent::ActionCompleted { action_id, result };
                    let verdict = self.evaluate(&event).await;
                    let decision = self.build_decision(verdict, &event);
                    self.execute(decision).await;
                    self.registry.gc();
                }
                Some(event) => {
                    let verdict = self.evaluate(&event).await;
                    let decision = self.build_decision(verdict, &event);
                    self.execute(decision).await;
                    self.registry.gc();
                }
                None => {
                    info!("event channel closed, shutting down");
                    self.shutdown().await;
                    break;
                }
            }
        }
        info!("mind stopped");
    }

    async fn evaluate(&self, event: &WakeEvent) -> MindVerdict {
        let prompt = self.build_mind_prompt(event).await;
        let event_desc = describe_event(event);

        let request = ChatRequest::new(&self.model, vec![
            Message::system(prompt),
            Message::user(event_desc),
        ])
        .with_tools(mind_tools())
        .with_tool_choice(ToolChoice::Required)
        .with_sampling(&self.sampling);

        let response = match self.provider.chat(&request).await {
            Ok(r) => r,
            Err(e) => {
                warn!(%e, "mind LLM call failed, falling back to respond");
                return MindVerdict::Respond;
            }
        };

        let thinking = response.text().unwrap_or("");
        if !thinking.is_empty() {
            info!(mind_thinking = %thinking, "mind internal thought");
        }

        let tool_calls = response.tool_calls();
        if tool_calls.is_empty() {
            warn!("mind returned no tool call despite Required, defaulting to respond");
            return MindVerdict::Respond;
        }

        let call = &tool_calls[0];
        let reason = call.arguments["reason"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let verdict = match call.name.as_str() {
            "respond" => {
                info!(reason = %reason, "mind decided: respond");
                MindVerdict::Respond
            }
            "drop" => {
                info!(reason = %reason, "mind decided: drop");
                MindVerdict::Drop
            }
            "defer" => {
                info!(reason = %reason, "mind decided: defer");
                MindVerdict::Defer
            }
            other => {
                warn!(tool = other, "mind called unknown tool, defaulting to respond");
                MindVerdict::Respond
            }
        };

        verdict
    }

    fn build_decision(&self, verdict: MindVerdict, event: &WakeEvent) -> MindDecision {
        if matches!(self.resolve_authority(event), Authority::Blocked) {
            info!("blocked person — dropping silently");
            return MindDecision::drop();
        }

        match verdict {
            MindVerdict::Drop | MindVerdict::Defer => MindDecision::drop(),
            MindVerdict::Respond => self.respond_to(event),
        }
    }

    fn resolve_authority(&self, event: &WakeEvent) -> Authority {
        let person = match event {
            WakeEvent::Message(msg) => msg.person.as_ref(),
            WakeEvent::IntentFired(intent) => intent.person.as_ref(),
            _ => None,
        };
        let personality = self.state.read_personality();
        person
            .and_then(|p| personality.relationships.get(p))
            .map_or(Authority::Default, |r| r.authority.clone())
    }

    fn respond_to(&self, event: &WakeEvent) -> MindDecision {
        let authority = self.resolve_authority(event);

        match event {
            WakeEvent::Message(msg) => {
                let conv_actions = self.registry.for_conversation(&msg.conversation);
                let running: Vec<&ActionState> = conv_actions
                    .iter()
                    .filter(|a| matches!(a.status, ActionStatus::Running))
                    .copied()
                    .collect();

                let unreplied: Vec<&ActionState> = running
                    .iter()
                    .filter(|a| !a.progress.read().map_or(false, |p| p.responded))
                    .copied()
                    .collect();

                if !unreplied.is_empty() {
                    if let Some(target) = unreplied.first() {
                        if target.inject_tx.is_some() {
                            return MindDecision::inject_one(target.id.clone(), msg.clone());
                        }
                    }
                    let cancel_ids: Vec<ActionId> =
                        unreplied.iter().map(|a| a.id.clone()).collect();
                    return MindDecision::cancel_and_spawn(
                        cancel_ids,
                        ActionRequest::respond(
                            vec![msg.clone()],
                            msg.conversation.clone(),
                            authority,
                        ),
                    );
                }

                if self.registry.at_capacity() {
                    if let Some(lowest) = self.registry.lowest_priority_running() {
                        if lowest.priority < ActionKind::Respond.default_priority() {
                            return MindDecision::cancel_and_spawn(
                                vec![lowest.id.clone()],
                                ActionRequest::respond(
                                    vec![msg.clone()],
                                    msg.conversation.clone(),
                                    authority,
                                ),
                            );
                        }
                    }
                    warn!("mind wants to respond but at capacity, dropping");
                    return MindDecision::drop();
                }

                MindDecision::spawn_one(ActionRequest::respond(
                    vec![msg.clone()],
                    msg.conversation.clone(),
                    authority,
                ))
            }
            WakeEvent::IdleTick { .. } => {
                if self.registry.at_capacity() {
                    return MindDecision::drop();
                }
                MindDecision::spawn_one(ActionRequest::ruminate())
            }
            WakeEvent::IntentFired(intent) => {
                if self.registry.at_capacity() {
                    return MindDecision::drop();
                }
                MindDecision::spawn_one(ActionRequest {
                    kind: ActionKind::Respond,
                    task: intent.task.clone(),
                    conversation: intent.conversation.clone(),
                    priority: ActionKind::Outreach.default_priority(),
                    messages: vec![],
                    timing: ActionTiming::Immediate,
                    context: None,
                    authority,
                })
            }
            WakeEvent::ActionCompleted { action_id, .. } => {
                let pending = self.registry.pending_after(action_id);
                let mut spawn = vec![];
                for pid in &pending {
                    if self.registry.all_dependencies_met(pid) {
                        if let Some(action) = self.registry.get(pid) {
                            spawn.push(ActionRequest {
                                kind: action.kind.clone(),
                                task: action.task.clone(),
                                conversation: action.conversation.clone(),
                                priority: action.priority,
                                messages: vec![],
                                timing: ActionTiming::Immediate,
                                context: None,
                                authority: Authority::Default,
                            });
                        }
                    }
                }
                if spawn.is_empty() {
                    MindDecision::drop()
                } else {
                    MindDecision {
                        spawn,
                        cancel: vec![],
                        supplement: vec![],
                        inject: vec![],
                    }
                }
            }
            WakeEvent::TypingUpdate { .. } => MindDecision::drop(),
            WakeEvent::Shutdown => MindDecision::drop(),
        }
    }

    async fn build_mind_prompt(&self, event: &WakeEvent) -> String {
        let mut prompt = String::with_capacity(1024);

        let query = crate::store::RecallQuery::by_text("my name, who I am", 1)
            .with_kind(crate::store::MemoryKind::Semantic)
            .with_min_importance(0.5);
        let identity = match self.store.recall(&query).await {
            Ok(memories) if !memories.is_empty() => memories[0].content.clone(),
            _ => "an unnamed being".into(),
        };

        {
            let personality = self.state.read_personality();

            prompt.push_str(&format!(
                "You are the inner mind of {}. You are the control gate — every event passes through you.\n",
                identity
            ));
            prompt.push_str("Evaluate what happened and decide whether to engage.\n\n");

            if let Some(msg) = event.message() {
                if let Some(person_id) = &msg.person {
                    if let Some(rel) = personality.relationships.get(person_id) {
                        prompt.push_str(&format!(
                            "## Person\n{} — {} (Authority: {})\nTrust: {:.0}%, Familiarity: {:.0}%\n\n",
                            person_id.0,
                            rel.label.as_str(),
                            rel.authority.as_str(),
                            rel.trust * 100.0,
                            rel.familiarity * 100.0,
                        ));
                    } else {
                        prompt.push_str(&format!(
                            "## Person\n{} — unknown (no relationship record)\n\n",
                            person_id.0,
                        ));
                    }
                }
            }
        }

        let running = self.registry.running_actions();
        prompt.push_str(&format!(
            "## State\nCapacity: {}/{}\n",
            running.len(),
            self.registry.max_concurrency(),
        ));
        if !running.is_empty() {
            prompt.push_str("Running actions:\n");
            for a in &running {
                let responded = a.progress.read().map_or(false, |p| p.responded);
                prompt.push_str(&format!(
                    "- {} ({:?}) conv={} responded={}\n",
                    a.id,
                    a.kind,
                    a.conversation.as_ref().map_or("none", |c| c.0.as_str()),
                    responded,
                ));
            }
        }
        prompt.push('\n');

        let recent_thoughts = self.store.recent_thoughts(5).await.unwrap_or_default();
        if !recent_thoughts.is_empty() {
            prompt.push_str("## Recent thoughts\n");
            for t in &recent_thoughts {
                prompt.push_str(&format!("- [{}] {}\n", t.kind.as_str(), t.content));
            }
            prompt.push('\n');
        }

        prompt.push_str("Use the respond, drop, or defer tool to make your decision.\n");

        prompt
    }

    fn gather_context(&self, cancelled_note: Option<String>) -> ActionContext {
        let concurrent_actions: Vec<ActionBrief> = self
            .registry
            .running_actions()
            .iter()
            .map(|a| ActionBrief {
                id: a.id.clone(),
                kind: a.kind.clone(),
                task: a.task.clone(),
                conversation: a.conversation.clone(),
            })
            .collect();

        ActionContext {
            cancelled_note,
            concurrent_actions,
        }
    }

    async fn handle_action_completed(
        &self,
        action_id: &ActionId,
        result: &super::action::ActionResult,
    ) {
        if let Some(ref delta) = result.delta {
            self.state.send_delta(delta.clone()).await;
            info!(%action_id, "forwarded personality delta to state task");
        }

        for msg in &result.unprocessed_messages {
            info!(%action_id, "re-queuing unprocessed message");
            self.event_tx
                .send(WakeEvent::Message(msg.clone()))
                .await
                .ok();
        }

        let action_conv = self
            .registry
            .get(action_id)
            .and_then(|a| a.conversation.clone());

        for msg in &result.injected_messages {
            if let Some(conv) = &action_conv {
                let recent = self
                    .store
                    .get_messages(conv, 5, None)
                    .await
                    .unwrap_or_default();

                let has_response_after = recent.iter().any(|m| {
                    matches!(m.role, MessageRole::Assistant) && m.timestamp > msg.timestamp
                });

                if !has_response_after {
                    info!(%action_id, "re-queuing injected message (no response found)");
                    self.event_tx
                        .send(WakeEvent::Message(msg.clone()))
                        .await
                        .ok();
                }
            } else {
                self.event_tx
                    .send(WakeEvent::Message(msg.clone()))
                    .await
                    .ok();
            }
        }
    }

    async fn execute(&mut self, decision: MindDecision) {
        for id in &decision.cancel {
            if self.registry.cancel(id) {
                info!(%id, "cancelled action");
            }
        }

        for (id, msg) in decision.inject {
            if let Some(action) = self.registry.get(&id) {
                if let Some(tx) = &action.inject_tx {
                    match tx.try_send(msg) {
                        Ok(()) => info!(%id, "injected message into running action"),
                        Err(e) => warn!(%id, %e, "failed to inject message"),
                    }
                }
            }
        }

        for request in decision.spawn {
            self.spawn_action(request).await;
        }

        for (id, ctx) in &decision.supplement {
            debug!(%id, note = %ctx.note, "supplementing action");
        }
    }

    async fn spawn_action(&mut self, mut request: ActionRequest) {
        let id = self.registry.next_id();
        let depends_on = match &request.timing {
            ActionTiming::Immediate => vec![],
            ActionTiming::After(dep) => vec![dep.clone()],
            ActionTiming::AfterAll(deps) => deps.clone(),
        };

        let is_pending = !depends_on.is_empty()
            && !depends_on.iter().all(|d| {
                self.registry
                    .get(d)
                    .map_or(true, |a| matches!(a.status, ActionStatus::Completed))
            });

        let status = if is_pending {
            ActionStatus::Pending
        } else {
            ActionStatus::Running
        };

        let kind = request.kind.clone();
        let task_desc = request.task.clone();
        let conversation = request.conversation.clone();
        let priority = request.priority;

        let progress = Arc::new(RwLock::new(ActionProgress::new()));

        let state = ActionState {
            id: id.clone(),
            kind: kind.clone(),
            task: task_desc.clone(),
            conversation,
            priority,
            status,
            has_responded: false,
            depends_on,
            handle: None,
            progress: progress.clone(),
            inject_tx: None,
        };

        self.registry.insert(state);

        if !is_pending {
            if request.context.is_none() {
                request.context = Some(self.gather_context(None));
            }
            self.launch_action_task(id, kind, task_desc, request).await;
        } else {
            info!(%id, task = %task_desc, "queued pending action");
        }
    }

    async fn launch_action_task(
        &mut self,
        id: ActionId,
        kind: ActionKind,
        task_desc: String,
        request: ActionRequest,
    ) {
        let (inject_tx, inject_rx) = mpsc::channel::<InboundMessage>(32);

        if let Some(action) = self.registry.get_mut(&id) {
            action.inject_tx = Some(inject_tx);
        }

        let event_tx = self.event_tx.clone();
        let action_id = id.clone();
        let state_handle = self.state.clone();
        let store = self.store.clone();
        let provider = self.provider.clone();
        let platform = self.platform.clone();
        let model = self.model.clone();
        let sampling = self.sampling.clone();
        let progress = self
            .registry
            .get(&id)
            .map(|a| a.progress.clone())
            .unwrap_or_else(|| Arc::new(RwLock::new(ActionProgress::new())));

        let context = request.context;

        let handle = tokio::spawn(async move {
            info!(%action_id, kind = ?kind, task = %task_desc, "action started");

            let ctx = super::session::SessionContext {
                action_id: action_id.clone(),
                kind,
                messages: request.messages,
                conversation: request.conversation,
                authority: request.authority,
                state: state_handle,
                store,
                provider,
                model,
                sampling,
                context,
                inject_rx,
                progress,
                max_turns: 10,
                platform,
                session_start: std::time::Instant::now(),
            };

            let result = super::session::run_session(ctx).await;

            if result.delta.is_some() {
                info!(%action_id, "action produced personality delta");
            }

            event_tx
                .send(WakeEvent::ActionCompleted {
                    action_id: action_id.clone(),
                    result,
                })
                .await
                .ok();

            info!(%action_id, "action completed");
        });

        if let Some(action) = self.registry.get_mut(&id) {
            action.handle = Some(handle);
        }
    }

    async fn shutdown(&mut self) {
        info!("mind shutting down, cancelling all actions");
        let running: Vec<ActionId> = self
            .registry
            .running_actions()
            .iter()
            .map(|a| a.id.clone())
            .collect();
        for id in &running {
            self.registry.cancel(id);
        }
    }
}

#[derive(Debug)]
enum MindVerdict {
    Respond,
    Drop,
    Defer,
}

fn describe_event(event: &WakeEvent) -> String {
    match event {
        WakeEvent::Message(msg) => {
            format!(
                "New message in conversation {}:\n{}",
                msg.conversation.0,
                msg.display_content()
            )
        }
        WakeEvent::IdleTick { elapsed_secs } => {
            format!("Idle tick. {:.0} seconds since last activity.", elapsed_secs)
        }
        WakeEvent::IntentFired(intent) => {
            let conv = intent
                .conversation
                .as_ref()
                .map_or("none".to_string(), |c| c.0.clone());
            format!(
                "Scheduled intent fired: {} (conversation: {})",
                intent.task, conv
            )
        }
        WakeEvent::ActionCompleted { action_id, result } => {
            let has_delta = result.delta.is_some();
            let unprocessed = result.unprocessed_messages.len();
            format!(
                "Action {} completed. personality_delta={} unprocessed_messages={}",
                action_id, has_delta, unprocessed
            )
        }
        WakeEvent::TypingUpdate {
            person, typing, ..
        } => {
            format!(
                "{} {} typing.",
                person.0,
                if *typing { "started" } else { "stopped" }
            )
        }
        WakeEvent::Shutdown => unreachable!(),
    }
}
