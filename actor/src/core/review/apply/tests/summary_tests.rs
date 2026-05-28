use super::*;

#[test]
fn summary_merge_rejects_trivial_and_appends_novel_fragments() {
    let existing = "Sam likes concise summaries and deployment updates.";

    assert_eq!(merge_summary_update(Some(existing), "short"), None);
    assert_eq!(
        merge_summary_update(Some(existing), "Sam likes concise summaries."),
        None
    );
    assert_eq!(
        merge_summary_update(Some(existing), "Keeps launch checklist.").as_deref(),
        Some("Sam likes concise summaries and deployment updates. Keeps launch checklist.")
    );
    assert_eq!(
        merge_summary_update(
            Some("Sam likes concise summaries and careful deployment updates"),
            "Keeps launch checklist."
        )
        .as_deref(),
        Some("Sam likes concise summaries and careful deployment updates. Keeps launch checklist.")
    );
    assert_eq!(
        merge_ordered_ids(
            vec!["msg-1".to_string(), "msg-2".to_string()],
            vec!["msg-2".to_string(), "msg-3".to_string()]
        ),
        vec![
            "msg-1".to_string(),
            "msg-2".to_string(),
            "msg-3".to_string()
        ]
    );
}
