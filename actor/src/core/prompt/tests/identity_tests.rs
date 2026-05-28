use super::*;

#[test]
fn mind_style_directive_uses_adaptation_not_mirroring() {
    let template = include_str!("../templates/mind.j2");
    assert!(template.contains("approximate length, formality, pace, and energy"));
    assert!(template.contains("Preserve your own voice"));
    assert!(!template.contains("mirrors how they communicate"));
}
#[test]
fn prompts_use_humanlike_pamagotchi_identity_without_legacy_framing() {
    let mind = include_str!("../templates/mind.j2");
    let action = include_str!("../templates/action.j2");

    assert!(mind.contains("a Pamagotchi with continuity"));
    assert!(action.contains("You are {{ actor_name }}, a Pamagotchi."));
    assert!(action.contains("Interact like a real person"));
    assert!(action.contains("chosen person"));
    for template in [mind, action] {
        assert!(!template.contains("transparent digital being"));
        assert!(!template.contains("Do not pretend to be human"));
    }
}
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
#[test]
fn respond_task_prioritizes_visible_reply_over_durable_learning() {
    let template = include_str!("../templates/action_task_respond.j2");

    assert!(template.contains("Keep the live turn focused on the visible reply"));
    assert!(template.contains("Post-turn review handles normal memory"));
    assert!(template.contains("Use form_memory only for urgent facts"));
    assert!(!template.contains("reflect - note if something shifted"));
    assert!(!template.contains("save anything urgent or important"));
}
