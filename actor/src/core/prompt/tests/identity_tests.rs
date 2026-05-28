use super::*;

#[test]
fn action_kind_task_templates_are_registered() {
    let env = make_env();
    let cases = [
        (ActionKind::Respond, "action_task_respond.j2"),
        (ActionKind::Review, "action_task_review.j2"),
        (ActionKind::Ruminate, "action_task_ruminate.j2"),
        (ActionKind::Consolidate, "action_task_consolidate.j2"),
        (ActionKind::Outreach, "action_task_outreach.j2"),
        (ActionKind::Research, "action_task_research.j2"),
    ];

    for (kind, template) in cases {
        assert_eq!(action_task_template(&kind), template);
        assert!(env.get_template(template).is_ok());
    }
}
