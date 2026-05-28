use super::focus::FocusManager;
use protocol::{
    ClientRequest, GatewayKindView, GatewayVarKind, GatewayVarSpec, GatewayView, ServerEvent,
    SubscriptionTopic,
};
use relay::ApiClient;

mod api;
mod gateways;
mod input;
mod scroll;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Chat,
    Settings,
    Gateways,
    GatewayDetail,
    Debug,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSelection {
    Gateways,
    Debug,
    Back,
}

pub struct ChatMessage {
    pub content: String,
    pub is_self: bool,
}

pub struct App {
    pub port: u16,
    pub screen: Screen,
    pub input: String,
    pub cursor: usize,
    pub input_scroll: usize,
    pub input_width: usize,
    pub messages: Vec<ChatMessage>,
    pub messages_scroll: usize,
    pub composing: bool,
    pub focus: FocusManager,
    pub settings_selection: SettingsSelection,
    pub api: Option<ApiClient>,
    pub gateways: Vec<GatewayView>,
    pub gateways_selection: usize,
    pub gateways_scroll: usize,
    pub show_add_dialog: bool,
    pub add_selection: usize,
    pub available_gateways: Vec<GatewayKindView>,
    pub selected_gateway_id: Option<String>,
    pub gateway_var_selection: usize,
    pub editing_gateway_var: bool,
    pub gateway_var_input: String,
    pub gateway_var_cursor: usize,
    pub debug_snapshot: String,
    pub debug_request_id: Option<String>,
    pub debug_scroll: usize,
}

impl App {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            screen: Screen::Chat,
            input: String::new(),
            cursor: 0,
            input_scroll: 0,
            input_width: 0,
            messages: Vec::new(),
            messages_scroll: 0,
            composing: false,
            focus: FocusManager::new(),
            settings_selection: SettingsSelection::Gateways,
            api: None,
            gateways: Vec::new(),
            gateways_selection: 0,
            gateways_scroll: 0,
            show_add_dialog: false,
            add_selection: 0,
            available_gateways: Vec::new(),
            selected_gateway_id: None,
            gateway_var_selection: 0,
            editing_gateway_var: false,
            gateway_var_input: String::new(),
            gateway_var_cursor: 0,
            debug_snapshot: "No debug snapshot loaded.".into(),
            debug_request_id: None,
            debug_scroll: 0,
        }
    }
}

pub fn gateway_var_input_value(gateway: &GatewayView, spec: &GatewayVarSpec) -> String {
    let value = gateway.vars.get(&spec.key).or(spec.default.as_ref());
    match spec.kind {
        GatewayVarKind::String => value
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        GatewayVarKind::Bool => value
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
            .to_string(),
        GatewayVarKind::StringList => value
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default(),
    }
}

fn gateway_var_value_from_input(spec: &GatewayVarSpec, input: &str) -> serde_json::Value {
    match spec.kind {
        GatewayVarKind::String => serde_json::Value::String(input.trim().to_string()),
        GatewayVarKind::Bool => serde_json::Value::Bool(matches!(
            input.trim().to_ascii_lowercase().as_str(),
            "true" | "yes" | "y" | "1" | "on"
        )),
        GatewayVarKind::StringList => serde_json::Value::Array(
            input
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| serde_json::Value::String(value.to_string()))
                .collect(),
        ),
    }
}

fn request_id(prefix: &str) -> String {
    format!(
        "{}-{}",
        prefix,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    )
}

fn wrapped_line_count(line: &str, width: usize) -> usize {
    if width == 0 || line.is_empty() {
        return 1;
    }
    let char_count = line.chars().count();
    (char_count + width - 1) / width
}

pub fn visual_line_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return text.matches('\n').count() + 1;
    }
    text.split('\n').map(|l| wrapped_line_count(l, width)).sum()
}

pub fn visual_cursor_y(text: &str, byte_offset: usize, width: usize) -> usize {
    let byte_offset = clamp_to_char_boundary(text, byte_offset);
    let before = &text[..byte_offset];
    if width == 0 {
        return before.matches('\n').count();
    }
    let lines: Vec<&str> = before.split('\n').collect();
    let mut y = 0;
    for (i, line) in lines.iter().enumerate() {
        if i < lines.len() - 1 {
            y += wrapped_line_count(line, width);
        } else {
            y += line.chars().count() / width;
        }
    }
    y
}

pub fn visual_cursor_x(text: &str, byte_offset: usize, width: usize) -> usize {
    let byte_offset = clamp_to_char_boundary(text, byte_offset);
    let before = &text[..byte_offset];
    let last_line = before.rsplit('\n').next().unwrap_or(before);
    let col = last_line.chars().count();
    if width == 0 { col } else { col % width }
}

fn clamp_to_char_boundary(text: &str, byte_offset: usize) -> usize {
    let mut offset = byte_offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn byte_offset_for_char_column(
    text: &str,
    line_start: usize,
    line_end: usize,
    column: usize,
) -> usize {
    text[line_start..line_end]
        .char_indices()
        .nth(column)
        .map(|(offset, _)| line_start + offset)
        .unwrap_or(line_end)
}

#[cfg(test)]
mod tests;
