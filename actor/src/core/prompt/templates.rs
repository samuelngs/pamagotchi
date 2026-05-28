use super::*;

pub(super) fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
    env.add_template("mind.j2", include_str!("templates/mind.j2"))
        .unwrap();
    env.add_template("action.j2", include_str!("templates/action.j2"))
        .unwrap();
    env.add_template(
        "action_task_respond.j2",
        include_str!("templates/action_task_respond.j2"),
    )
    .unwrap();
    env.add_template(
        "action_task_review.j2",
        include_str!("templates/action_task_review.j2"),
    )
    .unwrap();
    env.add_template(
        "action_task_ruminate.j2",
        include_str!("templates/action_task_ruminate.j2"),
    )
    .unwrap();
    env.add_template(
        "action_task_consolidate.j2",
        include_str!("templates/action_task_consolidate.j2"),
    )
    .unwrap();
    env.add_template(
        "action_task_outreach.j2",
        include_str!("templates/action_task_outreach.j2"),
    )
    .unwrap();
    env.add_template(
        "action_task_research.j2",
        include_str!("templates/action_task_research.j2"),
    )
    .unwrap();
    env
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
