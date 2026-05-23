use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alias {
    pub platform_id: String,
    pub external_id: String,
    pub display_name: String,
}
