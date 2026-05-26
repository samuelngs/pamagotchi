use super::ScreenAction;
use crate::tui::app::App;
use crate::tui::focus::FocusId;
use crate::tui::widgets::{Breadcrumb, Button, ShortKey};

use crossterm::event::{KeyCode, KeyEvent};
use protocol::{GatewayConnectionState, GatewaySetupInstructions, GatewayVarKind, GatewayVarSpec};
use ratatui::prelude::*;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);

    frame.render_widget(
        Breadcrumb {
            items: &["actor", "settings", "gateway(s)", "gateway"],
        },
        Rect::new(layout[0].x + 1, layout[0].y + 1, layout[0].width, 1),
    );

    render_body(frame, app, layout[0]);
    render_buttons(frame, app, layout[1]);
}

fn render_body(frame: &mut Frame, app: &mut App, area: Rect) {
    app.clamp_gateway_var_selection();

    let Some(gateway) = app.selected_gateway() else {
        frame.render_widget(
            Line::from("Gateway not found").style(Style::default().fg(Color::DarkGray)),
            Rect::new(area.x + 1, area.y + 3, area.width, 1),
        );
        return;
    };

    let mut y = area.y + 3;
    let status = format_connection_state(&gateway.connection_state);
    frame.render_widget(
        Line::from(format!("{} #{}", gateway.kind, gateway.id))
            .style(Style::default().fg(Color::White).bold()),
        Rect::new(area.x + 1, y, area.width, 1),
    );
    y += 2;

    frame.render_widget(
        Line::from(format!("status: {status}")).style(Style::default().fg(Color::DarkGray)),
        Rect::new(area.x + 1, y, area.width, 1),
    );
    y += 2;

    match &gateway.setup_instructions {
        Some(GatewaySetupInstructions::Text { title, body }) => {
            frame.render_widget(
                Line::from(title.as_str()).style(Style::default().fg(Color::White).bold()),
                Rect::new(area.x + 1, y, area.width, 1),
            );
            y += 1;
            frame.render_widget(
                Line::from(body.as_str()).style(Style::default().fg(Color::White)),
                Rect::new(area.x + 1, y, area.width, 1),
            );
            y += 2;
        }
        Some(GatewaySetupInstructions::QrCode {
            title,
            body,
            rendered,
            ..
        }) => {
            frame.render_widget(
                Line::from(title.as_str()).style(Style::default().fg(Color::White).bold()),
                Rect::new(area.x + 1, y, area.width, 1),
            );
            y += 1;
            frame.render_widget(
                Line::from(body.as_str()).style(Style::default().fg(Color::White)),
                Rect::new(area.x + 1, y, area.width, 1),
            );
            y += 2;
            render_qr(
                frame,
                rendered,
                Rect::new(
                    area.x + 1,
                    y,
                    area.width.saturating_sub(2),
                    area.height.saturating_sub(y - area.y),
                ),
            );
            y = area.y + area.height;
        }
        None => {
            frame.render_widget(
                Line::from("No setup required").style(Style::default().fg(Color::DarkGray)),
                Rect::new(area.x + 1, y, area.width, 1),
            );
            y += 2;
        }
    }

    render_vars(
        frame,
        app,
        Rect::new(
            area.x + 1,
            y,
            area.width.saturating_sub(2),
            area.height.saturating_sub(y - area.y),
        ),
    );
}

fn render_qr(frame: &mut Frame, rendered: &str, area: Rect) {
    let lines = rendered.lines().take(area.height as usize);
    for (idx, line) in lines.enumerate() {
        frame.render_widget(
            Line::from(line).style(Style::default().fg(Color::White)),
            Rect::new(area.x, area.y + idx as u16, area.width, 1),
        );
    }
}

fn render_buttons(frame: &mut Frame, app: &App, area: Rect) {
    let back_btn = Button {
        label: "back",
        shortkey: Some(ShortKey::Esc),
        focused: app.focus.is(FocusId::GatewayDetailBack),
    };
    let back_w = back_btn.width();
    frame.render_widget(back_btn, Rect::new(area.x, area.y, back_w, 1));

    let remove_btn = Button {
        label: "remove",
        shortkey: None,
        focused: app.focus.is(FocusId::GatewayDetailRemove),
    };
    let remove_w = remove_btn.width();
    let remove_x = area.x + back_w + 1;
    frame.render_widget(remove_btn, Rect::new(remove_x, area.y, remove_w, 1));

    let restart_btn = Button {
        label: "restart",
        shortkey: None,
        focused: app.focus.is(FocusId::GatewayDetailRestart),
    };
    let restart_w = restart_btn.width();
    let restart_x = remove_x + remove_w + 1;
    frame.render_widget(restart_btn, Rect::new(restart_x, area.y, restart_w, 1));
}

pub async fn handle_key(app: &mut App, key: KeyEvent) -> ScreenAction {
    if app.editing_gateway_var {
        return handle_var_edit_key(app, key).await;
    }

    let order = focus_order(app);
    match key.code {
        KeyCode::Esc => {
            if app.focus.is(FocusId::GatewayDetailBack) {
                ScreenAction::Back
            } else {
                app.focus.set(FocusId::GatewayDetailBack);
                ScreenAction::None
            }
        }
        KeyCode::Tab | KeyCode::Right => {
            app.focus.next_in(&order);
            ScreenAction::None
        }
        KeyCode::BackTab | KeyCode::Left => {
            app.focus.prev_in(&order);
            ScreenAction::None
        }
        KeyCode::Up => {
            if app.focus.is(FocusId::GatewayDetailVar) && app.gateway_var_selection > 0 {
                app.gateway_var_selection -= 1;
            } else {
                app.focus.prev_in(&order);
            }
            ScreenAction::None
        }
        KeyCode::Down => {
            if app.focus.is(FocusId::GatewayDetailVar)
                && app.gateway_var_selection + 1 < app.selected_gateway_var_specs().len()
            {
                app.gateway_var_selection += 1;
            } else {
                app.focus.next_in(&order);
            }
            ScreenAction::None
        }
        KeyCode::Enter => match app.focus.current() {
            FocusId::GatewayDetailBack => ScreenAction::Back,
            FocusId::GatewayDetailRemove => {
                app.remove_selected_gateway().await;
                ScreenAction::Back
            }
            FocusId::GatewayDetailRestart => {
                app.restart_selected_gateway().await;
                ScreenAction::None
            }
            FocusId::GatewayDetailVar => {
                match app.selected_gateway_var_spec().map(|spec| &spec.kind) {
                    Some(GatewayVarKind::Bool) => {
                        app.toggle_selected_gateway_bool_var().await;
                    }
                    Some(_) => {
                        app.begin_gateway_var_edit();
                    }
                    None => {}
                }
                ScreenAction::None
            }
            _ => ScreenAction::None,
        },
        _ => ScreenAction::None,
    }
}

async fn handle_var_edit_key(app: &mut App, key: KeyEvent) -> ScreenAction {
    match key.code {
        KeyCode::Esc => app.cancel_gateway_var_edit(),
        KeyCode::Enter => app.commit_gateway_var_edit().await,
        KeyCode::Backspace => app.delete_gateway_var_char(),
        KeyCode::Left => app.move_gateway_var_cursor_left(),
        KeyCode::Right => app.move_gateway_var_cursor_right(),
        KeyCode::Char(c) => app.insert_gateway_var_char(c),
        _ => {}
    }
    ScreenAction::None
}

fn focus_order(app: &App) -> Vec<FocusId> {
    let mut order = Vec::with_capacity(4);
    order.push(FocusId::GatewayDetailBack);
    if !app.selected_gateway_var_specs().is_empty() {
        order.push(FocusId::GatewayDetailVar);
    }
    order.push(FocusId::GatewayDetailRemove);
    order.push(FocusId::GatewayDetailRestart);
    order
}

fn render_vars(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }

    let Some(gateway) = app.selected_gateway() else {
        return;
    };
    let specs = app.selected_gateway_var_specs();
    if specs.is_empty() {
        return;
    }

    frame.render_widget(
        Line::from("vars").style(Style::default().fg(Color::White).bold()),
        Rect::new(area.x, area.y, area.width, 1),
    );

    for (idx, spec) in specs.iter().enumerate() {
        let y = area.y + 1 + idx as u16;
        if y >= area.y + area.height {
            break;
        }
        let focused = app.focus.is(FocusId::GatewayDetailVar) && app.gateway_var_selection == idx;
        let editing = focused && app.editing_gateway_var;
        let value = if editing {
            app.gateway_var_input.clone()
        } else {
            display_var_value(gateway, spec)
        };
        let marker = if focused { ">" } else { " " };
        let required = if spec.required { " *" } else { "" };
        let line = format!("{marker} {}{}: {value}", spec.label, required);
        let style = if focused {
            Style::default().fg(Color::Black).bg(Color::White)
        } else {
            Style::default().fg(Color::White)
        };
        frame.render_widget(
            Line::from(line).style(style),
            Rect::new(area.x, y, area.width, 1),
        );

        if editing {
            let cursor_x = area
                .x
                .saturating_add(2 + spec.label.len() as u16 + required.len() as u16 + 2)
                .saturating_add(app.gateway_var_cursor as u16)
                .min(area.x + area.width.saturating_sub(1));
            frame.set_cursor_position(Position::new(cursor_x, y));
        }
    }
}

fn display_var_value(gateway: &protocol::GatewayView, spec: &GatewayVarSpec) -> String {
    let value = crate::tui::app::gateway_var_input_value(gateway, spec);
    match spec.kind {
        GatewayVarKind::Bool => value,
        GatewayVarKind::String | GatewayVarKind::StringList if spec.secret && !value.is_empty() => {
            "********".into()
        }
        GatewayVarKind::String | GatewayVarKind::StringList if value.is_empty() => "(unset)".into(),
        GatewayVarKind::String | GatewayVarKind::StringList => value,
    }
}

fn format_connection_state(state: &GatewayConnectionState) -> &str {
    match state {
        GatewayConnectionState::NotConfigured => "not configured",
        GatewayConnectionState::SetupRequired => "setup required",
        GatewayConnectionState::Connecting => "connecting",
        GatewayConnectionState::Connected => "connected",
        GatewayConnectionState::Disconnected => "disconnected",
        GatewayConnectionState::Error { .. } => "error",
    }
}
