use super::super::decision::MindVerdict;
use inference::Tool;
use serde_json::{Value, json};

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "respond".into(),
            description: "Engage with this event. Start an action session to handle it.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "Why you decided to engage"
                    },
                    "style_directive": {
                        "type": "string",
                        "description": "Brief description of how this person communicates (tone, length, formality, language habits) so your response mirrors their style. Analyze their actual messages."
                    }
                },
                "required": ["reason"]
            }),
        },
        Tool {
            name: "drop".into(),
            description: "Ignore this event. Not worth engaging with right now.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "Why you decided to ignore"
                    }
                },
                "required": ["reason"]
            }),
        },
        Tool {
            name: "defer".into(),
            description:
                "Hold off on this event. Something else takes priority, or the timing isn't right."
                    .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "Why you decided to defer"
                    }
                },
                "required": ["reason"]
            }),
        },
    ]
}

pub fn execute(name: &str, args: &Value) -> Option<MindVerdict> {
    let reason = args["reason"].as_str().unwrap_or("").to_string();
    match name {
        "respond" => {
            let style_directive = args["style_directive"].as_str().map(|s| s.to_string());
            tracing::info!(reason = %reason, style = ?style_directive, "mind decided: respond");
            Some(MindVerdict::Respond { style_directive })
        }
        "drop" => {
            tracing::info!(reason = %reason, "mind decided: drop");
            Some(MindVerdict::Drop)
        }
        "defer" => {
            tracing::info!(reason = %reason, "mind decided: defer");
            Some(MindVerdict::Defer)
        }
        _ => None,
    }
}
