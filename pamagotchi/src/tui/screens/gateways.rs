use super::ScreenAction;
use crate::tui::app::App;
use crate::tui::focus::FocusId;
use crate::tui::widgets::{Breadcrumb, Button, ShortKey};

use crossterm::event::{KeyCode, KeyEvent};
use protocol::GatewayConnectionState;
use ratatui::prelude::*;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);

    frame.render_widget(
        Breadcrumb {
            items: &["actor", "settings", "gateway(s)"],
        },
        Rect::new(layout[0].x + 1, layout[0].y + 1, layout[0].width, 1),
    );

    if app.show_add_dialog {
        render_add_dialog(frame, app, layout[0]);
        let back_btn = Button {
            label: "back",
            shortkey: Some(ShortKey::Esc),
            focused: false,
        };
        let back_w = back_btn.width();
        frame.render_widget(back_btn, Rect::new(layout[1].x, layout[1].y, back_w, 1));
    } else {
        render_gateway_list(frame, app, layout[0]);
        render_buttons(frame, app, layout[1]);
    }
}

fn render_add_dialog(frame: &mut Frame, app: &App, area: Rect) {
    let prompt_y = area.y + 3;
    let prompt = Line::from("Add gateway").style(Style::default().fg(Color::White));
    frame.render_widget(prompt, Rect::new(area.x + 1, prompt_y, area.width, 1));

    if app.available_gateways.is_empty() {
        frame.render_widget(
            Line::from("No available gateways").style(Style::default().fg(Color::DarkGray)),
            Rect::new(area.x + 1, prompt_y + 2, area.width, 1),
        );
        return;
    }

    for (idx, gateway) in app.available_gateways.iter().enumerate() {
        let selectable = crate::tui::widgets::Selectable {
            label: &gateway.kind,
            shortkey: None,
            focused: app.focus.is(FocusId::GatewayAddKind) && app.add_selection == idx,
        };
        let w = selectable.width();
        frame.render_widget(
            selectable,
            Rect::new(area.x + 1, prompt_y + 2 + idx as u16, w, 1),
        );
    }
}

fn render_gateway_list(frame: &mut Frame, app: &App, area: Rect) {
    let start_y = area.y + 3;
    let max_visible = (area.height as usize).saturating_sub(3) - 1;

    if app.gateways.is_empty() {
        frame.render_widget(
            Line::from("(no gateways yet)").style(Style::default().fg(Color::DarkGray)),
            Rect::new(area.x + 1, start_y, area.width, 1),
        );
        return;
    }

    let scroll = app
        .gateways_scroll
        .min(app.gateways.len().saturating_sub(1));

    for i in 0..max_visible {
        let idx = scroll + i;
        if idx >= app.gateways.len() {
            break;
        }

        let y = start_y + i as u16;

        if let Some(gw) = app.gateways.get(idx) {
            let label = format_gateway_label(gw);
            let selectable = crate::tui::widgets::Selectable {
                label: &label,
                shortkey: None,
                focused: app.focus.is(FocusId::GatewayList) && app.gateways_selection == idx,
            };
            let w = selectable.width();
            frame.render_widget(selectable, Rect::new(area.x + 1, y, w, 1));
        }
    }
}

fn format_gateway_label(gw: &protocol::GatewayView) -> String {
    let state_str = format_connection_state(&gw.connection_state);
    format!("{} #{} [{state_str}]", gw.kind, gw.id)
}

fn format_connection_state(state: &GatewayConnectionState) -> &str {
    match state {
        GatewayConnectionState::NotConfigured => "not configured",
        GatewayConnectionState::SetupRequired => "setup required",
        GatewayConnectionState::Connecting => "connecting",
        GatewayConnectionState::Connected => "connected",
        GatewayConnectionState::Disconnected => "disconnected",
        GatewayConnectionState::Error { .. } => "Error",
    }
}

fn render_buttons(frame: &mut Frame, app: &App, area: Rect) {
    let back_btn = Button {
        label: "back",
        shortkey: Some(ShortKey::Esc),
        focused: app.focus.is(FocusId::GatewayBack),
    };
    let back_w = back_btn.width();
    frame.render_widget(back_btn, Rect::new(area.x, area.y, back_w, 1));

    let add_btn = Button {
        label: "add",
        shortkey: None,
        focused: app.focus.is(FocusId::GatewayAdd),
    };
    let add_w = add_btn.width();
    let add_x = area.x + back_w + 1;
    frame.render_widget(add_btn, Rect::new(add_x, area.y, add_w, 1));
}

pub async fn handle_key(app: &mut App, key: KeyEvent) -> ScreenAction {
    if app.show_add_dialog {
        return handle_add_dialog_key(app, key).await;
    }

    let order = focus_order(app);
    match key.code {
        KeyCode::Tab => {
            app.focus.next_in(&order);
            ScreenAction::None
        }
        KeyCode::BackTab => {
            app.focus.prev_in(&order);
            ScreenAction::None
        }
        KeyCode::Esc => {
            if app.focus.is(FocusId::GatewayBack) {
                return ScreenAction::Back;
            }
            app.focus.set(FocusId::GatewayBack);
            ScreenAction::None
        }
        KeyCode::Left => {
            app.focus.prev_in(&order);
            ScreenAction::None
        }
        KeyCode::Right => {
            app.focus.next_in(&order);
            ScreenAction::None
        }
        KeyCode::Up => {
            match app.focus.current() {
                FocusId::GatewayList if app.gateways_selection > 0 => {
                    app.gateways_selection -= 1;
                    if app.gateways_selection < app.gateways_scroll {
                        app.gateways_scroll = app.gateways_selection;
                    }
                }
                FocusId::GatewayBack | FocusId::GatewayAdd if !app.gateways.is_empty() => {
                    app.focus.set(FocusId::GatewayList);
                    app.gateways_selection = app.gateways.len().saturating_sub(1);
                }
                _ => app.focus.prev_in(&order),
            }
            ScreenAction::None
        }
        KeyCode::Down => {
            match app.focus.current() {
                FocusId::GatewayList if app.gateways_selection + 1 < app.gateways.len() => {
                    app.gateways_selection += 1;
                    let visible_area = area_visible_height();
                    if app.gateways_selection >= app.gateways_scroll + visible_area {
                        app.gateways_scroll =
                            app.gateways_selection.saturating_sub(visible_area - 1);
                    }
                }
                FocusId::GatewayList => app.focus.set(FocusId::GatewayBack),
                _ => app.focus.next_in(&order),
            }
            ScreenAction::None
        }
        KeyCode::Enter => match app.focus.current() {
            FocusId::GatewayAdd => {
                app.show_add_dialog = true;
                app.focus.set(FocusId::GatewayAddKind);
                app.add_selection = 0;
                ScreenAction::None
            }
            FocusId::GatewayBack => ScreenAction::Back,
            FocusId::GatewayList => {
                if let Some(gateway) = app.gateways.get(app.gateways_selection) {
                    app.selected_gateway_id = Some(gateway.id.clone());
                    ScreenAction::Navigate(crate::tui::app::Screen::GatewayDetail)
                } else {
                    ScreenAction::None
                }
            }
            _ => ScreenAction::None,
        },
        _ => ScreenAction::None,
    }
}

async fn handle_add_dialog_key(app: &mut App, key: KeyEvent) -> ScreenAction {
    match key.code {
        KeyCode::Esc => {
            app.show_add_dialog = false;
            app.focus.set(FocusId::GatewayAdd);
            ScreenAction::None
        }
        KeyCode::Enter => {
            app.add_gateway().await;
            app.focus.set(FocusId::GatewayAdd);
            ScreenAction::None
        }
        KeyCode::Up => {
            if app.add_selection > 0 {
                app.add_selection -= 1;
            }
            ScreenAction::None
        }
        KeyCode::Down => {
            if app.add_selection + 1 < app.available_gateways.len() {
                app.add_selection += 1;
            }
            ScreenAction::None
        }
        _ => ScreenAction::None,
    }
}

fn area_visible_height() -> usize {
    10usize
}

fn focus_order(app: &App) -> Vec<FocusId> {
    let mut order = Vec::with_capacity(3);
    if !app.gateways.is_empty() {
        order.push(FocusId::GatewayList);
    }
    order.push(FocusId::GatewayBack);
    order.push(FocusId::GatewayAdd);
    order
}
