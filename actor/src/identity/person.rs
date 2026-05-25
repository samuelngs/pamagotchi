use protocol::PersonId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Person {
    pub id: PersonId,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
}
