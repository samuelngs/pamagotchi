use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Identity {
    pub gateway_id: String,
    pub external_id: String,
    pub display_name: Option<String>,
}
