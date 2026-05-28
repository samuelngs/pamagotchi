use super::*;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "send_message".into(),
            description: "Send a message. Omit gateway_id and external_id to reply in the current conversation. Provide both to send to a specific destination (use get_person with include_identities=true to find allowed gateway identities).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The message text"
                    },
                    "gateway_id": {
                        "type": "string",
                        "description": "Gateway to send through (e.g. discord, telegram, whatsapp)"
                    },
                    "external_id": {
                        "type": "string",
                        "description": "Recipient's ID on that gateway. Must be paired with gateway_id."
                    },
                    "media_url": {
                        "type": "string",
                        "description": "URL of media to attach. Some gateways require media_asset_id instead."
                    },
                    "media_asset_id": {
                        "type": "string",
                        "description": "Stored media asset ID to attach. Required for WhatsApp media sends."
                    },
                    "media_type": {
                        "type": "string",
                        "enum": ["image", "video", "audio", "sticker", "file"],
                        "description": "Type of media attachment"
                    },
                    "mime_type": {
                        "type": "string",
                        "description": "MIME type of the media (e.g. image/png, video/mp4)"
                    },
                    "filename": {
                        "type": "string",
                        "description": "Filename for file attachments"
                    },
                    "attachments": {
                        "type": "array",
                        "description": "Media attachments to send. Use media_asset_id for stored assets, especially WhatsApp.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "media_asset_id": {
                                    "type": "string",
                                    "description": "Stored media asset ID to attach"
                                },
                                "media_url": {
                                    "type": "string",
                                    "description": "URL of media to attach for gateways that support URL attachments"
                                },
                                "media_type": {
                                    "type": "string",
                                    "enum": ["image", "video", "audio", "sticker", "file"],
                                    "description": "Type of media attachment"
                                },
                                "mime_type": {
                                    "type": "string",
                                    "description": "MIME type of the media"
                                },
                                "filename": {
                                    "type": "string",
                                    "description": "Filename for file attachments"
                                }
                            },
                            "required": ["media_type"]
                        }
                    }
                },
                "required": ["content"]
            }),
        },
        Tool {
            name: "read_messages".into(),
            description: "Read messages from a conversation. Use to access older history beyond what's in your current context. Internal background actions may omit conversation to read a bounded recent-conversation view.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "conversation": {
                        "type": "string",
                        "description": "Conversation ID. Defaults to current conversation."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max messages to return (default 10)",
                        "default": 10
                    },
                    "before": {
                        "type": "integer",
                        "description": "Unix timestamp. Only return messages before this time. Use to page backwards through history."
                    }
                }
            }),
        },
        Tool {
            name: "update_conversation_summary".into(),
            description: "Update the rolling summary for the current conversation. Use during review or consolidation after reading enough recent messages.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "conversation": {
                        "type": "string",
                        "description": "Conversation ID. Defaults to current conversation."
                    },
                    "summary": {
                        "type": "string",
                        "description": "Compact summary preserving important facts, decisions, open questions, commitments, emotional tone, and last visible response."
                    },
                    "covered_message_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Message ids covered by this summary. Use the top-level message_id values returned by read_messages. Defaults to the current action's messages."
                    }
                },
                "required": ["summary"]
            }),
        },
    ]
}
