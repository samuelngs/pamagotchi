use super::ScreenAction;
use crate::tui::app::App;
use crate::tui::focus::FocusId;
use crate::tui::widgets::{Breadcrumb, Button, ShortKey};

use crossterm::event::{KeyCode, KeyEvent};
use protocol::{GatewayConnectionState, GatewaySetupInstructions};
use ratatui::prelude::*;

const FOCUS_ORDER: &[FocusId] = &[
    FocusId::GatewayDetailBack,
    FocusId::GatewayDetailRemove,
    FocusId::GatewayDetailRestart,
];

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

fn render_body(frame: &mut Frame, app: &App, area: Rect) {
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
        }
        None => {
            frame.render_widget(
                Line::from("No setup required").style(Style::default().fg(Color::DarkGray)),
                Rect::new(area.x + 1, y, area.width, 1),
            );
        }
    }
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
            app.focus.next_in(FOCUS_ORDER);
            ScreenAction::None
        }
        KeyCode::BackTab | KeyCode::Left => {
            app.focus.prev_in(FOCUS_ORDER);
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
            _ => ScreenAction::None,
        },
        _ => ScreenAction::None,
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
