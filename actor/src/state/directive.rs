use super::RelationshipStanding;
use protocol::{ChannelId, GroupId, PersonId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BehaviorDirective {
    pub id: String,
    pub scope: DirectiveScope,
    pub directive: String,
    pub set_by: PersonId,
    pub priority: i32,
    pub active: bool,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DirectiveScope {
    Global,
    Person(PersonId),
    RelationshipStanding(RelationshipStanding),
    Channel(ChannelId),
    Group(GroupId),
}

impl DirectiveScope {
    pub fn scope_type(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Person(_) => "person",
            Self::RelationshipStanding(_) => "relationship_standing",
            Self::Channel(_) => "channel",
            Self::Group(_) => "group",
        }
    }

    pub fn scope_value(&self) -> Option<String> {
        match self {
            Self::Global => None,
            Self::Person(p) => Some(p.0.clone()),
            Self::RelationshipStanding(a) => Some(a.as_str().to_string()),
            Self::Channel(c) => Some(c.0.clone()),
            Self::Group(g) => Some(g.0.clone()),
        }
    }
}
