use super::*;

pub(super) fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
    add(&mut env, "mind.j2", include_str!("templates/mind.j2"));
    add(&mut env, "action.j2", include_str!("templates/action.j2"));
    add(
        &mut env,
        "partials/action/persona.j2",
        include_str!("templates/partials/action/persona.j2"),
    );
    add(
        &mut env,
        "partials/action/current_context.j2",
        include_str!("templates/partials/action/current_context.j2"),
    );
    add(
        &mut env,
        "partials/action/relationship_rituals.j2",
        include_str!("templates/partials/action/relationship_rituals.j2"),
    );
    add(
        &mut env,
        "partials/action/adoption_ritual.j2",
        include_str!("templates/partials/action/adoption_ritual.j2"),
    );
    add(
        &mut env,
        "partials/action/review_transcript.j2",
        include_str!("templates/partials/action/review_transcript.j2"),
    );
    add(
        &mut env,
        "partials/action/memory_context.j2",
        include_str!("templates/partials/action/memory_context.j2"),
    );
    add(
        &mut env,
        "partials/action/writing_style.j2",
        include_str!("templates/partials/action/writing_style.j2"),
    );
    add(
        &mut env,
        "partials/action/runtime_context.j2",
        include_str!("templates/partials/action/runtime_context.j2"),
    );
    add(
        &mut env,
        "partials/action/identity_and_tools.j2",
        include_str!("templates/partials/action/identity_and_tools.j2"),
    );
    add(
        &mut env,
        "partials/mind/current_context.j2",
        include_str!("templates/partials/mind/current_context.j2"),
    );
    add(
        &mut env,
        "partials/mind/social_memory_context.j2",
        include_str!("templates/partials/mind/social_memory_context.j2"),
    );
    add(
        &mut env,
        "partials/mind/decision_protocol.j2",
        include_str!("templates/partials/mind/decision_protocol.j2"),
    );
    add(
        &mut env,
        "action_task_respond.j2",
        include_str!("templates/action_task_respond.j2"),
    );
    add(
        &mut env,
        "action_task_review.j2",
        include_str!("templates/action_task_review.j2"),
    );
    add(
        &mut env,
        "action_task_ruminate.j2",
        include_str!("templates/action_task_ruminate.j2"),
    );
    add(
        &mut env,
        "action_task_consolidate.j2",
        include_str!("templates/action_task_consolidate.j2"),
    );
    add(
        &mut env,
        "action_task_outreach.j2",
        include_str!("templates/action_task_outreach.j2"),
    );
    add(
        &mut env,
        "action_task_research.j2",
        include_str!("templates/action_task_research.j2"),
    );
    env
}

fn add(env: &mut Environment<'static>, name: &'static str, source: &'static str) {
    env.add_template(name, source).unwrap();
}

pub(super) fn action_task_template(kind: &ActionKind) -> &'static str {
    match kind {
        ActionKind::Respond => "action_task_respond.j2",
        ActionKind::Review => "action_task_review.j2",
        ActionKind::Ruminate => "action_task_ruminate.j2",
        ActionKind::Consolidate => "action_task_consolidate.j2",
        ActionKind::Outreach => "action_task_outreach.j2",
        ActionKind::Research => "action_task_research.j2",
    }
}
