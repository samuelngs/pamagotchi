use protocol::PersonId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Person {
    pub id: PersonId,
    pub name: String,
    pub bio: String,
    pub first_seen: i64,
    pub last_seen: i64,
}
