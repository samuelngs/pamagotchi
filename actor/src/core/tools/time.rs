use inference::Tool;
use serde_json::{json, Value};
use super::context::SessionContext;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "get_current_time".into(),
            description: "Get the current date and time. Optionally provide a timezone for local time conversion.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "timezone": {
                        "type": "string",
                        "description": "IANA timezone name (e.g. 'Asia/Hong_Kong', 'America/New_York', 'Europe/London'). If omitted, returns UTC only."
                    }
                }
            }),
        },
        Tool {
            name: "get_session_elapsed".into(),
            description: "Check how much time has passed since this session started. Useful to gauge pacing and how long you've been working.".into(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
        },
    ]
}

pub fn current_time(args: &Value) -> String {
    use chrono::Utc;

    let now = Utc::now();
    let mut out = format!("UTC: {}", now.format("%Y-%m-%d %H:%M:%S"));

    if let Some(tz_str) = args["timezone"].as_str() {
        match tz_str.parse::<chrono_tz::Tz>() {
            Ok(tz) => {
                let local = now.with_timezone(&tz);
                out.push_str(&format!(
                    "\nLocal ({}): {}",
                    tz_str,
                    local.format("%Y-%m-%d %H:%M:%S %Z")
                ));
            }
            Err(_) => {
                out.push_str(&format!("\nUnknown timezone: {tz_str}"));
            }
        }
    }

    out
}

pub fn session_elapsed(ctx: &SessionContext) -> String {
    let elapsed = ctx.session_start.elapsed();
    let secs = elapsed.as_secs();
    if secs < 60 {
        format!("{secs} seconds")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
    }
}
