pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

pub enum ToolChoice {
    Auto,
    None,
    Required,
}
