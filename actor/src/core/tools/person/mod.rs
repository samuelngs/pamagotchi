mod helpers;
mod identity;
mod links;
mod profile;

pub use identity::{request_identity_verification, resolve_identity_verification};
pub use links::{detach_profile, reject_profile_person_link};
pub use profile::{get, update};

use inference::Tool;
use serde_json::json;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "update_person".into(),
            description: "Update a person's name or summary. Use after learning someone's name or after building a clearer picture of who they are.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ref": {
                        "type": "string",
                        "description": "Person ref handle (e.g. x7Kp2mQ). Defaults to current conversation partner."
                    },
                    "name": {
                        "type": "string",
                        "description": "Person's name. Set when you learn it."
                    },
                    "summary": {
                        "type": "string",
                        "description": "Compressed impression — who they are, what they care about, how they communicate. Overwrites previous summary."
                    }
                }
            }),
        },
        Tool {
            name: "get_person".into(),
            description: "Look up a person's current profile — name, summary, first/last seen. Set include_identities=true only when you need attached gateway identities; for privacy, only the owner or that same person can see them. Use request_identity_verification instead when someone claims to be a known person on another platform.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ref": {
                        "type": "string",
                        "description": "Person ref handle. Defaults to current conversation partner."
                    },
                    "include_identities": {
                        "type": "boolean",
                        "description": "Include attached gateway identities. Defaults to false and is allowed only for self or owner."
                    }
                }
            }),
        },
        Tool {
            name: "request_identity_verification".into(),
            description: "Start verification when the current profile claims to be a known person on another platform. Creates a pending claim and asks the known person's existing identities to confirm before linking profiles.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "claimed_person": {
                        "type": "string",
                        "description": "Person ref for the existing known person being claimed."
                    }
                },
                "required": ["claimed_person"]
            }),
        },
        Tool {
            name: "resolve_identity_verification".into(),
            description: "Confirm or deny a pending identity verification request. Use only when the current conversation partner is the known person who was asked to confirm.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "claim": {
                        "type": "string",
                        "description": "Pending identity claim ID. If omitted, uses the newest pending claim for the current person."
                    },
                    "confirmed": {
                        "type": "boolean",
                        "description": "true if the current person confirms the claimant is really them; false if denied."
                    }
                },
                "required": ["confirmed"]
            }),
        },
        Tool {
            name: "detach_profile_from_person".into(),
            description: "Detach a profile from a person grouping without deleting profile memories. Use when a same-person association was wrong or no longer trusted.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "profile": {
                        "type": "string",
                        "description": "Profile ID to detach."
                    },
                    "person": {
                        "type": "string",
                        "description": "Person ID to detach from. Defaults to the current conversation person."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Why the link is being detached."
                    }
                },
                "required": ["profile"]
            }),
        },
        Tool {
            name: "reject_profile_person_link".into(),
            description: "Record that a profile should not be linked to a person. This preserves audit history and blocks weak repeated same-person assumptions.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "profile": {
                        "type": "string",
                        "description": "Profile ID to reject."
                    },
                    "person": {
                        "type": "string",
                        "description": "Person ID the profile should not be associated with."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Why the link is rejected."
                    }
                },
                "required": ["profile", "person"]
            }),
        },
    ]
}
