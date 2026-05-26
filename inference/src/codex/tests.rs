use super::events::{CodexEvent, CodexItemDetails};
use super::prompt::prompt_from_request;
use crate::{ChatRequest, ContentPart, Message, Tool};

#[test]
fn parse_agent_message_event() {
    let event: CodexEvent = serde_json::from_str(
        r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"hello"}}"#,
    )
    .unwrap();

    match event {
        CodexEvent::ItemCompleted { item } => match item.details {
            CodexItemDetails::AgentMessage { text } => assert_eq!(text, "hello"),
            _ => panic!("wrong item"),
        },
        _ => panic!("wrong event"),
    }
}

#[test]
fn parse_error_event() {
    let event: CodexEvent =
        serde_json::from_str(r#"{"type":"error","message":"no auth"}"#).unwrap();

    match event {
        CodexEvent::Error(error) => assert_eq!(error.message, "no auth"),
        _ => panic!("wrong event"),
    }
}

#[test]
fn prompt_includes_messages_and_tools() {
    let request = ChatRequest::new(
        "gpt-5",
        vec![
            Message::system("be brief"),
            Message::user_content(vec![
                ContentPart::text("look"),
                ContentPart::image_url("data:image/png;base64,abc"),
            ]),
        ],
    )
    .with_tools(vec![Tool {
        name: "remember".into(),
        description: "Store memory".into(),
        parameters: serde_json::json!({"type":"object"}),
    }]);

    let prompt = prompt_from_request(&request);
    assert!(prompt.contains("## System"));
    assert!(prompt.contains("[image: data:image/png;base64,abc]"));
    assert!(prompt.contains("remember"));
}
