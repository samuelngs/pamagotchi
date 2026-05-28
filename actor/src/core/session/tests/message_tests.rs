use super::*;

#[test]
fn message_metadata_embeds_attachments() {
    let metadata = message_metadata(&inbound(serde_json::json!({ "sender": "user" })));

    assert_eq!(metadata["sender"], "user");
    assert_eq!(metadata["attachments"][0]["kind"], "Sticker");
    assert_eq!(metadata["attachments"][0]["asset_id"], "media-1");
    assert_eq!(metadata["attachments"][0]["mime"], "image/webp");
}
#[test]
fn visual_attachments_require_vision() {
    let mut msg = inbound(Value::Null);
    msg.attachments[0].kind = MediaKind::Video;

    assert_eq!(required_capabilities(&[msg], &[]), vec![Capability::Vision]);
}
#[test]
fn file_attachments_do_not_require_vision() {
    let mut msg = inbound(Value::Null);
    msg.attachments[0].kind = MediaKind::File;

    assert!(required_capabilities(&[msg], &[]).is_empty());
}
#[test]
fn action_outcome_carries_review_artifacts() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let ctx = test_context(store, text_inbound("msg-1", "hello"));
    let thought = Thought {
        timestamp: 1000,
        kind: ThoughtKind::Observation,
        content: "Sam sounded rushed.".into(),
        importance: 0.8,
        confidence: 0.7,
        action_id: Some("action-test".into()),
        memories_accessed: vec![MemoryId("memory-recalled".into())],
        subjects: vec![],
    };
    let formed = MemoryId("memory-formed".into());
    let recalled = MemoryId("memory-recalled".into());
    let state = SessionState {
        responded: true,
        attempted_send: true,
        composing_released: false,
        delta: empty_delta(None),
        thoughts: vec![thought.clone()],
        memories_formed: vec![formed.clone()],
        recalled_memory_ids: vec![recalled.clone()],
        injected_messages: vec![],
        presented_injected_messages: vec![],
        presented_read_messages: vec![],
        pending_injected_messages: vec![],
        source_message_keys: Default::default(),
        queued_injected_message_keys: Default::default(),
        presented_injected_message_keys: Default::default(),
        applied_review_keys: Default::default(),
        presented_injection_count: 0,
    };

    let result = build_result(ctx, state, None, 1, false);

    match result {
        SessionResult::Action(outcome) => {
            assert!(outcome.responded);
            assert_eq!(outcome.thoughts.len(), 1);
            assert_eq!(outcome.thoughts[0].content, thought.content);
            assert_eq!(outcome.memories_formed, vec![formed]);
            assert_eq!(outcome.recalled_memory_ids, vec![recalled]);
        }
        SessionResult::Mind(_) => panic!("expected action outcome"),
    }
}
#[test]
fn prompt_snapshot_redacts_prompt_text_tool_payloads_and_images() {
    let messages = vec![
        Message::system("System prompt with profile context."),
        Message::User(UserMessage::Content(vec![
            ContentPart::text("Please inspect this image."),
            ContentPart::image_url("data:image/png;base64,secret-bytes"),
        ])),
        Message::Assistant(AssistantMessage {
            text: Some("I will send the update.".into()),
            reasoning_content: Some("private reasoning".into()),
            tool_calls: vec![ToolCall {
                id: "call-1".into(),
                name: "send_message".into(),
                arguments: serde_json::json!({
                    "content": "Private reply text.",
                    "external_id": "target-external-id",
                    "metadata": {
                        "safe": true
                    }
                }),
            }],
        }),
        Message::tool_result(
            "call-1",
            r#"{"messages":[{"content":"private readback"}],"result":"Message sent."}"#,
        ),
    ];

    let snapshot = prompt_snapshot_messages(&messages);

    assert_eq!(snapshot[0]["content"], "[redacted]");
    assert_eq!(
        snapshot[0]["content_len"],
        "System prompt with profile context.".len()
    );
    assert_eq!(snapshot[1]["content"], "[redacted]");
    assert_eq!(snapshot[1]["content_parts"][0]["content"], "[redacted]");
    assert_eq!(
        snapshot[1]["content_parts"][0]["content_len"],
        "Please inspect this image.".len()
    );
    assert_eq!(
        snapshot[1]["content_parts"][1]["url"],
        "[inline image redacted]"
    );
    assert_eq!(snapshot[2]["content"], "[redacted]");
    assert_eq!(snapshot[2]["content_len"], "I will send the update.".len());
    assert_eq!(
        snapshot[2]["tool_calls"][0]["arguments"]["content"],
        "[redacted]"
    );
    assert_eq!(
        snapshot[2]["tool_calls"][0]["arguments"]["external_id"],
        "[redacted]"
    );
    assert_eq!(
        snapshot[2]["tool_calls"][0]["arguments"]["metadata"]["safe"],
        true
    );
    assert_eq!(snapshot[2]["reasoning_len"], "private reasoning".len());
    assert!(snapshot[2].get("reasoning_content").is_none());
    assert_eq!(
        snapshot[3]["content"]["messages"][0]["content"],
        "[redacted]"
    );
}
#[tokio::test]
async fn injected_messages_dedupe_by_source_id_not_text() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let source = text_inbound("msg-1", "same text");
    let ctx = test_context(store.clone(), source.clone());
    let mut state = SessionState {
        responded: false,
        attempted_send: false,
        composing_released: false,
        delta: empty_delta(None),
        thoughts: vec![],
        memories_formed: vec![],
        recalled_memory_ids: vec![],
        injected_messages: vec![],
        presented_injected_messages: vec![],
        presented_read_messages: vec![],
        pending_injected_messages: vec![],
        source_message_keys: source_message_keys(&ctx.messages),
        queued_injected_message_keys: Default::default(),
        presented_injected_message_keys: Default::default(),
        applied_review_keys: Default::default(),
        presented_injection_count: 0,
    };
    let mut llm_messages = vec![Message::system("system"), Message::user("same text")];

    let injected_a = text_inbound("msg-2", "same text");
    let injected_b = text_inbound("msg-3", "same text");
    let duplicate_injected_a = text_inbound("msg-2", "same text");
    let duplicate_source = text_inbound("msg-1", "same text");

    assert!(remember_injected_message(&mut state, injected_a));
    assert!(remember_injected_message(&mut state, injected_b));
    assert!(!remember_injected_message(&mut state, duplicate_injected_a));
    assert!(!remember_injected_message(&mut state, duplicate_source));

    inject_pending_messages(&ctx, &mut state, &mut llm_messages).await;

    let same_text_user_messages = llm_messages
        .iter()
        .filter(
            |message| matches!(message, Message::User(user) if user.display_text() == "same text"),
        )
        .count();
    assert_eq!(same_text_user_messages, 3);
    assert_eq!(state.presented_injection_count, 2);
    assert_eq!(state.presented_injected_messages.len(), 2);
    assert_eq!(state.presented_injected_messages[0].message_id, "msg-2");
    assert_eq!(state.presented_injected_messages[1].message_id, "msg-3");
    assert!(state.pending_injected_messages.is_empty());

    let stored = store
        .get_messages(&source.conversation, 10, None)
        .await
        .unwrap();
    let source_ids = stored
        .iter()
        .filter(|message| matches!(message.role, MessageRole::User))
        .filter_map(|message| message.source_message_id.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(source_ids, vec!["msg-2", "msg-3"]);
}
