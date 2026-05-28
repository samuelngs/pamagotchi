use super::*;

#[tokio::test]
async fn action_transcript_records_run_turn_tools_messages_and_review_watermark() {
    let store = test_store();
    let action_id = "action-1";

    store
        .start_action_run(&ActionRunRecord {
            action_id: action_id.into(),
            kind: "respond".into(),
            task: "Respond to message".into(),
            conversation: Some(ConversationId("relay:local".into())),
            started_at: 1000,
            ended_at: None,
            status: "running".into(),
            responded: false,
            attempts: 0,
        })
        .await
        .unwrap();
    store
        .append_action_message(&ActionMessageRecord {
            action_id: action_id.into(),
            role: "user".into(),
            conversation: Some(ConversationId("relay:local".into())),
            source_gateway_id: Some("relay".into()),
            source_message_id: Some("msg-1".into()),
            sender_external_id: Some("local".into()),
            reply_external_id: Some("local".into()),
            content: Some("hello".into()),
            created_at: 1001,
        })
        .await
        .unwrap();
    store
        .append_action_message(&ActionMessageRecord {
            action_id: action_id.into(),
            role: "user".into(),
            conversation: Some(ConversationId("relay:local".into())),
            source_gateway_id: Some("relay".into()),
            source_message_id: Some("msg-1".into()),
            sender_external_id: Some("local".into()),
            reply_external_id: Some("local".into()),
            content: Some("duplicate delivery".into()),
            created_at: 1002,
        })
        .await
        .unwrap();
    store
        .append_action_turn(&ActionTurnRecord {
            action_id: action_id.into(),
            turn: 0,
            attempt: 1,
            prompt_hash: "abc123".into(),
            model: Some("model-a".into()),
            finish: Some("tool_calls".into()),
            input_tokens: Some(10),
            output_tokens: Some(3),
            text_len: 4,
            reasoning_len: 0,
            tool_call_count: 1,
            created_at: 1002,
        })
        .await
        .unwrap();
    store
        .record_prompt_snapshot(&ActionPromptSnapshotRecord {
            action_id: action_id.into(),
            turn: 0,
            attempt: 1,
            prompt_hash: "abc123".into(),
            messages: serde_json::json!([
                {
                    "role": "system",
                    "content": "System prompt with current profile context."
                },
                {
                    "role": "user",
                    "content": "hello"
                }
            ]),
            created_at: 1002,
        })
        .await
        .unwrap();
    store
        .append_tool_call(&ToolCallRecord {
            action_id: action_id.into(),
            turn: 0,
            call_id: "call-1".into(),
            name: "send_message".into(),
            args: serde_json::json!({"content": "hi"}),
            result: serde_json::json!({"result": "Message sent."}),
            success: true,
            started_at: 1003,
            ended_at: 1004,
        })
        .await
        .unwrap();
    store
        .finish_action_run(
            action_id,
            1005,
            "completed",
            true,
            1,
            vec![MemoryId("memory-formed".into())],
            vec![MemoryId("memory-recalled".into())],
        )
        .await
        .unwrap();

    assert!(
        store
            .mark_review_scheduled(action_id, "review-1", 1006)
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_review_scheduled(action_id, "review-2", 1007)
            .await
            .unwrap()
    );
    assert!(store.action_review_scheduled(action_id).await.unwrap());
    store
        .record_review_output(&ReviewOutputAudit {
            id: "review-output-1".into(),
            review_action_id: "review-1".into(),
            source_action_id: Some(action_id.into()),
            input: serde_json::json!({
                "memories": [{
                    "content": "Sam prefers concise summaries.",
                    "evidence_message_ids": ["msg-1"]
                }]
            }),
            result: serde_json::json!({
                "status": "applied",
                "memories": 1,
                "skipped": []
            }),
            applied_at: 1007,
        })
        .await
        .unwrap();

    let conn = store.lock().unwrap();
    let (status, responded, attempts): (String, i32, u32) = conn
        .query_row(
            "SELECT status, responded, attempts FROM action_runs WHERE action_id = ?1",
            params![action_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "completed");
    assert_eq!(responded, 1);
    assert_eq!(attempts, 1);

    let tool_count: u32 = conn
        .query_row(
            "SELECT count(*) FROM action_tool_calls WHERE action_id = ?1",
            params![action_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tool_count, 1);

    let message_count: u32 = conn
        .query_row(
            "SELECT count(*) FROM action_messages WHERE action_id = ?1",
            params![action_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(message_count, 1);
    drop(conn);

    let transcript = store.action_transcript(action_id).await.unwrap();
    let run = transcript.run.expect("action run");
    assert_eq!(run.status, "completed");
    assert!(run.responded);
    assert_eq!(run.attempts, 1);
    assert_eq!(
        transcript.memories_formed,
        vec![MemoryId("memory-formed".into())]
    );
    assert_eq!(
        transcript.recalled_memory_ids,
        vec![MemoryId("memory-recalled".into())]
    );
    assert_eq!(transcript.messages.len(), 1);
    assert_eq!(transcript.messages[0].content.as_deref(), Some("hello"));
    assert_eq!(transcript.turns.len(), 1);
    assert_eq!(transcript.turns[0].model.as_deref(), Some("model-a"));
    assert_eq!(transcript.prompt_snapshots.len(), 1);
    assert_eq!(transcript.prompt_snapshots[0].prompt_hash, "abc123");
    assert_eq!(
        transcript.prompt_snapshots[0].messages[1]["content"],
        "hello"
    );
    assert_eq!(transcript.tool_calls.len(), 1);
    assert_eq!(transcript.tool_calls[0].name, "send_message");
    assert_eq!(transcript.tool_calls[0].result["result"], "Message sent.");
    let review_outputs = store.review_outputs_for_action("review-1").await.unwrap();
    assert_eq!(review_outputs.len(), 1);
    assert_eq!(
        review_outputs[0].source_action_id.as_deref(),
        Some(action_id)
    );
    assert_eq!(
        review_outputs[0].input["memories"][0]["content"],
        "[redacted]"
    );
    assert_eq!(
        review_outputs[0].input["memories"][0]["evidence_message_ids"][0],
        "msg-1"
    );
    assert_eq!(review_outputs[0].result["memories"], 1);

    let source_review_outputs = store
        .review_outputs_for_source_action(action_id)
        .await
        .unwrap();
    assert_eq!(source_review_outputs.len(), 1);
    assert_eq!(source_review_outputs[0].review_action_id, "review-1");
}

#[tokio::test]
async fn tool_call_transcripts_redact_sensitive_args_and_results() {
    let store = test_store();
    store
        .append_tool_call(&ToolCallRecord {
            action_id: "action-redact".into(),
            turn: 0,
            call_id: "call-redact".into(),
            name: "get_person".into(),
            args: serde_json::json!({
                "include_identities": true,
                "delivery_required": true,
                "reason": "deliver to target",
                "external_id": "target-external-id",
            }),
            result: serde_json::json!({
                "result": serde_json::json!({
                    "identities": [{
                        "gateway_id": "discord",
                        "external_id": "target-external-id",
                        "display_name": "Target"
                    }],
                    "messages": [{
                        "content": "private deployment detail"
                    }]
                })
                .to_string()
            }),
            success: true,
            started_at: 1000,
            ended_at: 1001,
        })
        .await
        .unwrap();

    let conn = store.lock().unwrap();
    let (args_json, result_json): (String, String) = conn
        .query_row(
            "SELECT args_json, result_json FROM action_tool_calls WHERE action_id = ?1",
            params!["action-redact"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let args: serde_json::Value = serde_json::from_str(&args_json).unwrap();
    let result: serde_json::Value = serde_json::from_str(&result_json).unwrap();
    let inner_result: serde_json::Value =
        serde_json::from_str(result["result"].as_str().unwrap()).unwrap();

    assert_eq!(args["external_id"], "[redacted]");
    assert_eq!(args["reason"], "[redacted]");
    assert_eq!(inner_result["identities"][0]["external_id"], "[redacted]");
    assert_eq!(inner_result["messages"][0]["content"], "[redacted]");
    assert_eq!(inner_result["identities"][0]["gateway_id"], "discord");
}
