mod helpers;
mod identity;
mod links;
mod profile;
mod social;

pub use identity::{request_identity_verification, resolve_identity_verification};
pub use links::{detach_profile, reject_profile_person_link};
pub use profile::{get, update, update_profile};
pub use social::upsert_social_relation;

use inference::Tool;
use serde_json::json;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "update_profile".into(),
            description: "Update the current account-specific profile. Use for display name, profile summary, and communication style learned in this gateway/profile boundary.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ref": {
                        "type": "string",
                        "description": "Profile ref handle. Defaults to the current profile."
                    },
                    "display_name": {
                        "type": "string",
                        "description": "Observed display name for this profile/account."
                    },
                    "summary": {
                        "type": "string",
                        "description": "Rich account-specific summary: stable facts, interests, preferences, relationship context, and important social understanding for this profile. Overwrites previous profile summary."
                    },
                    "comm_style": {
                        "type": "string",
                        "description": "Communication style and addressing preferences for this profile: tone, message length, formality, casing, punctuation, language patterns, emoji use, and preferred name or form of address. Overwrites previous profile style."
                    }
                }
            }),
        },
        Tool {
            name: "update_person".into(),
            description: "Update a verified/likely same-human person grouping. Use profile updates first; update person fields only when evidence supports a cross-profile person-level fact or style.".into(),
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
                        "description": "Rich person-level summary: stable facts, interests, preferences, relationship context, and important social understanding that should apply across verified/likely profiles. It may mention communication preferences when they are stable person-level context, but keep detailed style in comm_style. Overwrites previous summary."
                    },
                    "comm_style": {
                        "type": "string",
                        "description": "Person-level communication style and addressing preferences that are supported across profiles or explicitly confirmed. Profile comm_style remains the more specific source. Overwrites previous person style."
                    }
                }
            }),
        },
        Tool {
            name: "get_person".into(),
            description: "Look up a person's current profile — name, summary, first/last seen. Set include_identities=true only when you need attached gateway identities; for privacy, only the chosen person or that same person can see them. Use request_identity_verification instead when someone claims to be a known person on another platform.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ref": {
                        "type": "string",
                        "description": "Person ref handle. Defaults to current conversation partner."
                    },
                    "include_identities": {
                        "type": "boolean",
                        "description": "Include attached gateway identities. Defaults to false and is allowed only for self or chosen person."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Required when include_identities=true. Explain why gateway identities are needed so the request is auditable in the action transcript."
                    },
                    "delivery_required": {
                        "type": "boolean",
                        "description": "Set true only when full external ids are needed to deliver an allowed message. Defaults to false, which returns masked external ids.",
                        "default": false
                    }
                }
            }),
        },
        Tool {
            name: "request_identity_verification".into(),
            description: "Start verification when the current profile claims to be a known person on another platform. Creates a pending claim and asks the known person's existing identities to confirm only when evidence is strong enough. Chosen person, restricted, and blocked targets require chosen-person confirmation before any contact.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "claimed_person": {
                        "type": "string",
                        "description": "Person ref for the existing known person being claimed."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Short evidence-bound reason from the current conversation, e.g. what the person explicitly claimed."
                    },
                    "evidence": {
                        "type": "string",
                        "enum": ["self_declaration", "chosen_person_vouched", "mutual_claim", "shared_knowledge", "configured_identity"],
                        "description": "Type of evidence supporting this verification request. Defaults to self_declaration. chosen_person_vouched and configured_identity require chosen-person authority; self_declaration records the claim without contacting anyone."
                    }
                },
                "required": ["claimed_person", "reason"]
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
        Tool {
            name: "upsert_social_relation".into(),
            description: "Record or update an evidence-backed social graph relation between two person ids. Use during review/consolidation or chosen-person-directed maintenance, not as casual response behavior.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "person_a": {
                        "type": "string",
                        "description": "First person id in the relation."
                    },
                    "person_b": {
                        "type": "string",
                        "description": "Second person id in the relation."
                    },
                    "relation": {
                        "type": "string",
                        "enum": ["parent", "child", "sibling", "partner", "coworker", "friend"],
                        "description": "Relation type. Use a custom lowercase string only if none of the listed values apply."
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["a_to_b", "b_to_a", "bidirectional"],
                        "description": "Direction semantics for the relation. Use a_to_b when person_a has the relation to person_b, b_to_a for the reverse, and bidirectional for symmetric relations such as sibling, partner, coworker, or friend. Defaults from relation type."
                    },
                    "confidence": {
                        "type": "number",
                        "description": "0.0 to 1.0 confidence based on evidence.",
                        "default": 0.5
                    },
                    "status": {
                        "type": "string",
                        "enum": ["hypothesis", "stated", "confirmed", "denied", "outdated"],
                        "description": "Evidence status for the social relation.",
                        "default": "stated"
                    },
                    "source_kind": {
                        "type": "string",
                        "enum": ["inferred", "stated", "chosen_person_confirmed", "import", "system"],
                        "description": "How the relation was sourced. chosen_person_confirmed requires chosen-person authority.",
                        "default": "stated"
                    },
                    "asserted_by_person_id": {
                        "type": "string",
                        "description": "Person id of the speaker/source who asserted this relation. Defaults to the cited current-conversation speaker for stated or chosen-person-confirmed relations."
                    },
                    "evidence": {
                        "type": "object",
                        "description": "Compact supporting evidence such as quotes, message ids, or review rationale."
                    },
                    "evidence_message_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Exact message ids supporting this relation."
                    },
                    "evidence_quote": {
                        "type": "string",
                        "description": "Short quote or paraphrase supporting this relation."
                    }
                },
                "required": ["person_a", "person_b", "relation"]
            }),
        },
    ]
}

#[cfg(test)]
mod tests;
