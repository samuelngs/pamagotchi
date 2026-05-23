#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Clone)]
pub enum ToolChoice {
    Auto,
    None,
    Required,
}
