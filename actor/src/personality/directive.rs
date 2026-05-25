use protocol::{GroupId, PersonId};
use super::{Authority, Label};
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
    Label(Label),
    Authority(Authority),
    Group(GroupId),
}

impl DirectiveScope {
    pub fn scope_type(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Person(_) => "person",
            Self::Label(_) => "label",
            Self::Authority(_) => "authority",
            Self::Group(_) => "group",
        }
    }

    pub fn scope_value(&self) -> Option<String> {
        match self {
            Self::Global => None,
            Self::Person(p) => Some(p.0.clone()),
            Self::Label(l) => Some(l.as_str().to_string()),
            Self::Authority(a) => Some(a.as_str().to_string()),
            Self::Group(g) => Some(g.0.clone()),
        }
    }
}
