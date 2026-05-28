use super::*;

pub(super) fn parse_attachments(args: &Value) -> Result<Vec<MediaAttachment>, String> {
    if let Some(items) = args["attachments"].as_array() {
        let mut attachments = Vec::with_capacity(items.len());
        for item in items {
            if let Some(attachment) = parse_attachment(item)? {
                attachments.push(attachment);
            }
        }
        return Ok(attachments);
    }

    parse_attachment(args).map(|attachment| attachment.into_iter().collect())
}

fn parse_attachment(value: &Value) -> Result<Option<MediaAttachment>, String> {
    let Some(kind_str) = value["media_type"].as_str() else {
        return Ok(None);
    };
    let Some(kind) = MediaKind::parse(kind_str) else {
        return Err(format!("Unknown media type: {kind_str}"));
    };

    let asset_id = value["media_asset_id"]
        .as_str()
        .map(|id| MediaAssetId(id.to_string()));
    let url = if asset_id.is_some() {
        None
    } else {
        value["media_url"].as_str().map(String::from)
    };

    if asset_id.is_none() && url.is_none() {
        return Ok(None);
    }

    Ok(Some(MediaAttachment {
        kind,
        asset_id,
        url,
        mime: value["mime_type"].as_str().map(String::from),
        filename: value["filename"].as_str().map(String::from),
        size: None,
    }))
}

pub(super) fn outbound_metadata(attachments: &[MediaAttachment]) -> Value {
    if attachments.is_empty() {
        Value::Null
    } else {
        serde_json::json!({ "attachments": attachments })
    }
}
