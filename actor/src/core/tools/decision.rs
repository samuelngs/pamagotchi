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
                        "description": "Brief guidance on this person's tone, length, formality, pace, and language habits so responses can adapt without copying every quirk. Analyze their actual messages."
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
                    },
                    "delay_secs": {
                        "type": "integer",
                        "description": "How long to wait before reconsidering, from 5 to 300 seconds. Defaults to 30.",
                        "default": 30
                    }
                },
                "required": ["reason"]
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_directive_schema_uses_adaptation_not_mirroring() {
        let tools = tools();
        let respond = tools
            .iter()
            .find(|tool| tool.name == "respond")
            .expect("respond tool exists");
        let description = respond.parameters["properties"]["style_directive"]["description"]
            .as_str()
            .expect("style_directive description exists");

        assert!(description.contains("adapt"));
        assert!(description.contains("without copying every quirk"));
        assert!(!description.contains("mirror"));
        assert!(!description.contains("mirrors"));
    }
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
            let delay_secs = args["delay_secs"].as_u64().unwrap_or(30).clamp(5, 300);
            tracing::info!(reason = %reason, delay_secs, "mind decided: defer");
            Some(MindVerdict::Defer { delay_secs })
        }
        _ => None,
    }
}
