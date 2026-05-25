use crate::state::Authority;
use crate::store::MemorySource;
use protocol::MemoryId;
use serde_json::Value;
use super::context::SessionContext;

pub async fn check(
    name: &str,
    args: &Value,
    ctx: &SessionContext,
) -> Result<(), String> {
    match name {
        "form_memory" => {
            let tags: Vec<&str> = args["tags"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            let has_identity_tag = tags.iter().any(|t| {
                let lower = t.to_lowercase();
                lower == "identity" || lower == "self" || lower == "name"
            });
            if has_identity_tag && !matches!(ctx.authority, Authority::Owner) {
                return Err("Something feels wrong about this. You don't want to change something this core about yourself.".into());
            }
        }
        "forget_memory" => {
            let id = args["memory_id"].as_str().unwrap_or("");
            if let Ok(Some(mem)) = ctx.store.get_memory(&MemoryId(id.to_string())).await {
                if matches!(mem.source, MemorySource::External)
                    && !matches!(ctx.authority, Authority::Owner)
                {
                    return Err(
                        "This memory feels fundamental — you instinctively hold onto it."
                            .into(),
                    );
                }
            }
        }
        "reflect" => {
            if let Some(rels) = args["relationship_changes"].as_array() {
                for r in rels {
                    if r.get("authority").is_some()
                        && !matches!(ctx.authority, Authority::Owner)
                    {
                        return Err(
                            "Changing how you feel about someone isn't something you'd do on command."
                                .into(),
                        );
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}
