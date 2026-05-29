use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(GatewayId);
id_type!(ChannelId);
id_type!(SpaceId);
id_type!(MessageId);
id_type!(IdentityId);
id_type!(ProfileId);
id_type!(PersonId);
id_type!(ConversationId);
id_type!(MemoryId);
id_type!(MediaAssetId);

// Retained temporarily for semantic/social-group code that is unrelated to gateway routing.
// Gateway routing must use ChannelId/SpaceId instead.
id_type!(GroupId);

static GENERATED_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn identity_id(gateway_id: &GatewayId, external_id: &str) -> IdentityId {
    IdentityId(deterministic_id(
        "identity",
        gateway_id.as_str(),
        external_id,
    ))
}

pub fn channel_id(gateway_id: &GatewayId, external_id: &str) -> ChannelId {
    ChannelId(deterministic_id(
        "channel",
        gateway_id.as_str(),
        external_id,
    ))
}

pub fn space_id(gateway_id: &GatewayId, external_id: &str) -> SpaceId {
    SpaceId(deterministic_id("space", gateway_id.as_str(), external_id))
}

pub fn inbound_message_id(channel_id: &ChannelId, platform_message_id: &str) -> MessageId {
    MessageId(deterministic_id(
        "message",
        channel_id.as_str(),
        &format!("{platform_message_id}\0inbound"),
    ))
}

pub fn generated_conversation_id() -> ConversationId {
    ConversationId(generated_id("conversation"))
}

pub fn generated_message_id() -> MessageId {
    MessageId(generated_id("message"))
}

pub fn deterministic_id(prefix: &str, left: &str, right: &str) -> String {
    let mut bytes = Vec::with_capacity(left.len() + right.len() + 1);
    bytes.extend_from_slice(left.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(right.as_bytes());
    format!("{prefix}:{}", stable_hash_hex(&bytes))
}

fn generated_id(prefix: &str) -> String {
    let counter = GENERATED_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!(
        "{prefix}:{}",
        stable_hash_hex(format!("{nanos}\0{counter}").as_bytes())
    )
}

fn stable_hash_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
