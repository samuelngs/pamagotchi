use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PersonId(pub String);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Person {
    pub id: PersonId,
    pub name: String,
    pub bio: String,
    pub first_seen: i64,
    pub last_seen: i64,
}
