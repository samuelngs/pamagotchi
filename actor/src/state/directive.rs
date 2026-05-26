use super::Authority;
use protocol::{GroupId, PersonId};
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
    Authority(Authority),
    Group(GroupId),
}

impl DirectiveScope {
    pub fn scope_type(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Person(_) => "person",
            Self::Authority(_) => "authority",
            Self::Group(_) => "group",
        }
    }

    pub fn scope_value(&self) -> Option<String> {
        match self {
            Self::Global => None,
            Self::Person(p) => Some(p.0.clone()),
            Self::Authority(a) => Some(a.as_str().to_string()),
            Self::Group(g) => Some(g.0.clone()),
        }
    }
}
