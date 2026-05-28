use super::*;

#[tokio::test]
async fn collect_preserves_malformed_tool_arguments_as_error() {
    let (tx, rx) = mpsc::channel(4);
    tx.send(Ok(StreamEvent::ToolCallBegin {
        index: 0,
        id: "call-1".into(),
        name: "send_message".into(),
    }))
    .await
    .unwrap();
    tx.send(Ok(StreamEvent::ToolCallDelta {
        index: 0,
        arguments_delta: "{\"content\":".into(),
    }))
    .await
    .unwrap();
    drop(tx);

    let response = ChatStream::new(rx).collect().await.unwrap();
    let args = &response.message.tool_calls[0].arguments;

    assert_eq!(args["__invalid_tool_json"], true);
    assert_eq!(args["raw_arguments"], "{\"content\":");
}
