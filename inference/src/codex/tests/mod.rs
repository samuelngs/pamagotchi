use super::app_server::{
    AppServerSession, dynamic_tools, parse_dynamic_tool_call, tool_response, turn_start_params,
};
use super::events::{AppServerEventState, handle_notification, parse_notification};
use super::options::{CodexEffort, CodexOptions};
use super::prompt::prompt_from_request;
use crate::{AppServerToolResult, AppServerToolResultContent};
use crate::{ChatRequest, ContentPart, Message, StreamEvent, Tool};
use std::path::Path;
use tokio::sync::mpsc;

#[tokio::test]
async fn app_server_agent_message_delta_emits_text() {
    let value = serde_json::json!({
        "method": "item/agentMessage/delta",
        "params": {
            "threadId": "thread",
            "turnId": "turn",
            "itemId": "msg",
            "delta": "hello"
        }
    });
    let notification = parse_notification(value).unwrap();
    let (tx, mut rx) = mpsc::channel(1);
    let mut state = AppServerEventState::new(true);

    handle_notification(notification, &tx, &mut state)
        .await
        .unwrap();
    drop(tx);

    match rx.recv().await.unwrap().unwrap() {
        StreamEvent::TextDelta(text) => assert_eq!(text, "hello"),
        _ => panic!("expected text delta"),
    }
    assert_eq!(state.final_text(), "hello");
}

#[tokio::test]
async fn app_server_token_usage_uses_last_turn_usage() {
    let value = serde_json::json!({
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "thread",
            "turnId": "turn",
            "tokenUsage": {
                "last": {
                    "inputTokens": 11,
                    "cachedInputTokens": 2,
                    "outputTokens": 7,
                    "reasoningOutputTokens": 3,
                    "totalTokens": 18
                },
                "total": {
                    "inputTokens": 20,
                    "cachedInputTokens": 2,
                    "outputTokens": 10,
                    "reasoningOutputTokens": 3,
                    "totalTokens": 30
                }
            }
        }
    });
    let notification = parse_notification(value).unwrap();
    let (tx, mut rx) = mpsc::channel(1);
    let mut state = AppServerEventState::new(true);

    handle_notification(notification, &tx, &mut state)
        .await
        .unwrap();
    drop(tx);

    match rx.recv().await.unwrap().unwrap() {
        StreamEvent::Usage(usage) => {
            assert_eq!(usage.input_tokens, 11);
            assert_eq!(usage.output_tokens, 7);
        }
        _ => panic!("expected usage"),
    }
}

#[test]
fn app_server_unknown_notification_without_params_is_ignored() {
    let notification = parse_notification(serde_json::json!({
        "method": "remoteControl/status/changed"
    }))
    .unwrap();

    assert!(matches!(
        notification,
        super::events::AppServerNotification::Other
    ));
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

#[test]
fn app_server_command_uses_isolated_noninteractive_flags() {
    let session = AppServerSession::new(CodexOptions {
        model: "gpt-5".into(),
        command: "codex".into(),
        cwd: Some("/tmp/work".into()),
        profile_v2: Some("pamagotchi".into()),
        sandbox: Some("read-only".into()),
        effort: None,
        extra_args: vec!["--strict-config".into()],
    });

    let cmd = session.build_command(Path::new("/tmp/pamagotchi-codex-home"));
    let args = cmd
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert_eq!(args[0], "app-server");
    assert!(args.windows(2).any(|pair| pair == ["--listen", "stdio://"]));
    assert!(
        args.windows(2)
            .any(|pair| pair == ["-c", "approval_policy=\"never\""])
    );
    assert!(args.windows(2).any(|pair| pair == ["--disable", "hooks"]));
    assert!(args.windows(2).any(|pair| pair == ["--disable", "plugins"]));
    assert!(args.windows(2).any(|pair| pair == ["--disable", "apps"]));
    assert!(
        args.windows(2)
            .any(|pair| pair == ["--disable", "memories"])
    );
    assert!(args.contains(&"--strict-config".into()));
    assert!(!args.contains(&"--ignore-user-config".into()));
    assert!(!args.contains(&"--profile-v2".into()));
}

#[test]
fn app_server_dynamic_tools_match_protocol_shape() {
    let tools = dynamic_tools(&[Tool {
        name: "send_message".into(),
        description: "Send a message".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "content": {"type": "string"}
            },
            "required": ["content"]
        }),
    }]);

    assert_eq!(
        tools,
        serde_json::json!([{
            "name": "send_message",
            "description": "Send a message",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "content": {"type": "string"}
                },
                "required": ["content"]
            }
        }])
    );
}

#[test]
fn app_server_tool_call_request_and_response_match_protocol_shape() {
    let call = parse_dynamic_tool_call(serde_json::json!({
        "threadId": "thread",
        "turnId": "turn",
        "callId": "call_1",
        "namespace": "pamagotchi",
        "tool": "send_message",
        "arguments": {"content": "yo"}
    }))
    .unwrap();

    assert_eq!(call.id, "call_1");
    assert_eq!(call.name, "send_message");
    assert_eq!(call.namespace.as_deref(), Some("pamagotchi"));
    assert_eq!(call.arguments, serde_json::json!({"content": "yo"}));

    let response = tool_response(AppServerToolResult {
        success: true,
        content: vec![
            AppServerToolResultContent::Text("sent".into()),
            AppServerToolResultContent::ImageUrl("data:image/png;base64,aaa".into()),
        ],
    });

    assert_eq!(
        response,
        serde_json::json!({
            "success": true,
            "contentItems": [
                {"type": "inputText", "text": "sent"},
                {"type": "inputImage", "imageUrl": "data:image/png;base64,aaa"}
            ]
        })
    );
}

#[test]
fn options_default_to_codex_spark_model() {
    let options: CodexOptions = serde_json::from_value(serde_json::json!({})).unwrap();

    assert_eq!(options.model, "gpt-5.3-codex-spark");
    assert_eq!(options.command, "codex");
    assert_eq!(options.profile_v2.as_deref(), Some("pamagotchi"));
    assert_eq!(options.sandbox.as_deref(), Some("read-only"));
    assert_eq!(options.effort, None);
}

#[test]
fn options_parse_app_server_effort() {
    let options: CodexOptions =
        serde_json::from_value(serde_json::json!({"effort": "medium"})).unwrap();

    assert_eq!(options.effort, Some(CodexEffort::Medium));
}

#[test]
fn options_reject_unknown_app_server_effort() {
    let err = serde_json::from_value::<CodexOptions>(serde_json::json!({
        "effort": "maximum"
    }))
    .unwrap_err();

    assert!(err.to_string().contains("unknown variant"));
}

#[test]
fn app_server_turn_start_uses_configured_effort() {
    let options = CodexOptions {
        model: "gpt-5".into(),
        command: "codex".into(),
        cwd: None,
        profile_v2: Some("pamagotchi".into()),
        sandbox: Some("read-only".into()),
        effort: Some(CodexEffort::High),
        extra_args: Vec::new(),
    };
    let request = ChatRequest::new("gpt-5", Vec::new());

    let params = turn_start_params(&options, &request, "hello", "thread_1").unwrap();

    assert_eq!(params["threadId"], "thread_1");
    assert_eq!(params["model"], "gpt-5");
    assert_eq!(params["effort"], "high");
}
