use super::truncate;
use serde_json::json;

#[test]
fn truncate_handles_multilingual_multibyte_cutoff_boundaries() {
    let cases = [
        ("abc回def", 4, "abc回..."),
        ("abéçd", 3, "abé..."),
        ("hi🙂there", 3, "hi🙂..."),
        ("abcمdef", 4, "abcم..."),
        ("abनमस्ते", 3, "abन..."),
    ];

    for (value, max, expected) in cases {
        assert!(
            !value.is_char_boundary(max),
            "{value:?} byte cutoff {max} should be inside a multibyte character"
        );
        assert_eq!(truncate(value, max), expected);
    }
}

#[test]
fn truncate_keeps_untruncated_multibyte_text_without_ellipsis() {
    let value = "回家";

    assert_eq!(truncate(value, 2), value);
    assert_eq!(truncate(value, 3), value);
}

#[test]
fn truncate_handles_apply_review_payload_with_traditional_chinese_summary() {
    let payload = (0..3)
        .find_map(|padding| {
            let summary = format!(
                "{}{}",
                "a".repeat(padding),
                "回顧部署後續事項，保留繁體中文摘要。".repeat(20)
            );
            let payload = json!({
                "conversation_summary": {
                    "conversation_id": "relay:local",
                    "summary": summary,
                    "covered_message_ids": ["msg-traditional-chinese"]
                },
                "memories": []
            })
            .to_string();
            (!payload.is_char_boundary(200)).then_some(payload)
        })
        .expect("test payload should place byte 200 inside a multibyte character");

    let truncated = truncate(&payload, 200);

    assert!(truncated.ends_with("..."));
    assert!(truncated.is_char_boundary(truncated.len()));
}
