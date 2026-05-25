use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alias {
    pub gateway_id: String,
    pub external_id: String,
    pub display_name: String,
}
