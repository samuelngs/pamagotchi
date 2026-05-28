use super::*;

pub(super) fn finish_reason_name(reason: &FinishReason) -> &'static str {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::ToolCalls => "tool_calls",
        FinishReason::Length => "length",
        FinishReason::ContentFilter => "content_filter",
    }
}

pub(super) fn build_result(
    mut ctx: SessionContext,
    mut state: SessionState,
    verdict: Option<MindVerdict>,
    attempts: usize,
    cancelled: bool,
) -> SessionResult {
    match ctx.kind {
        SessionKind::Mind => {
            if cancelled {
                SessionResult::Mind(MindVerdict::Drop)
            } else {
                SessionResult::Mind(verdict.unwrap_or(MindVerdict::Respond {
                    style_directive: None,
                }))
            }
        }
        SessionKind::Action(_) => {
            while let Ok(msg) = ctx.inject_rx.try_recv() {
                remember_injected_message(&mut state, msg);
            }
            let pending = std::mem::take(&mut state.pending_injected_messages);
            let delta = if !cancelled && tools::has_changes(&state.delta) {
                Some(state.delta)
            } else {
                None
            };
            SessionResult::Action(Outcome {
                responded: !cancelled && state.responded,
                attempted_send: state.attempted_send,
                cancelled,
                delta,
                pending_messages: pending,
                review_messages: state.presented_injected_messages,
                thoughts: state.thoughts,
                memories_formed: state.memories_formed,
                recalled_memory_ids: state.recalled_memory_ids,
                had_injections: state.presented_injection_count > 0,
                attempts: attempts as u32,
            })
        }
    }
}
