use super::*;

pub(super) fn thread_start_params(options: &CodexOptions, request: &ChatRequest) -> Value {
    let mut params = json!({
        "model": request.model,
        "approvalPolicy": "never",
        "approvalsReviewer": "user",
        "ephemeral": true,
        "sandbox": options.sandbox.as_deref().unwrap_or("read-only"),
    });
    if let Some(cwd) = &options.cwd {
        params["cwd"] = Value::String(cwd.clone());
    }
    if !request.tools.is_empty() {
        params["dynamicTools"] = dynamic_tools(&request.tools);
    }
    params
}

pub(super) fn turn_start_params(
    options: &CodexOptions,
    request: &ChatRequest,
    prompt: &str,
    thread_id: &str,
) -> anyhow::Result<Value> {
    let mut params = json!({
        "threadId": thread_id,
        "input": [{"type": "text", "text": prompt}],
        "model": request.model,
        "approvalPolicy": "never",
    });
    if let Some(cwd) = &options.cwd {
        params["cwd"] = Value::String(cwd.clone());
    }
    Ok(params)
}
