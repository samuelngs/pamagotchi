use super::*;

pub(in crate::codex) fn dynamic_tools(tools: &[crate::Tool]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.parameters,
                })
            })
            .collect(),
    )
}

pub(in crate::codex) fn parse_dynamic_tool_call(
    params: Value,
) -> anyhow::Result<AppServerToolCall> {
    let params: DynamicToolCallParams =
        serde_json::from_value(params).context("failed to parse codex dynamic tool call")?;
    Ok(AppServerToolCall {
        id: params.call_id,
        name: params.tool,
        arguments: params.arguments,
        namespace: params.namespace,
    })
}

pub(in crate::codex) fn tool_response(result: AppServerToolResult) -> Value {
    let content_items = result
        .content
        .into_iter()
        .map(|content| match content {
            AppServerToolResultContent::Text(text) => {
                json!({ "type": "inputText", "text": text })
            }
            AppServerToolResultContent::ImageUrl(image_url) => {
                json!({ "type": "inputImage", "imageUrl": image_url })
            }
        })
        .collect::<Vec<_>>();
    json!({
        "success": result.success,
        "contentItems": content_items,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DynamicToolCallParams {
    arguments: Value,
    call_id: String,
    #[serde(default)]
    namespace: Option<String>,
    tool: String,
}
