// ============================================================================
// Concrete Route Implementations
// ============================================================================

use audido_core::modules::core::CoreHandle;
use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};

use crate::{
    router::{RouteAction, RouteHandler},
    routes::eq::EqualizerRoute,
    state::AppState,
    states::{EqState, SettingsOption, SettingsState, normalizer::NormalizerState},
};

/// Settings route
#[derive(Debug, Clone)]
pub struct SettingsRoute;

impl RouteHandler for SettingsRoute {
    fn render(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        draw_settings_panel(frame, area, &state.settings, &state.eq, &state.normalizer);
    }

    fn handle_input(
        &mut self,
        key: KeyCode,
        state: &mut AppState,
        __handle: &CoreHandle,
    ) -> anyhow::Result<RouteAction> {
        match key {
            KeyCode::Up => state.settings.prev_item(),
            KeyCode::Down => state.settings.next_item(),
            KeyCode::Enter => {
                // Navigate to EQ panel
                return Ok(RouteAction::Push(Box::new(EqualizerRoute::default())));
            }
            _ => {}
        }
        Ok(RouteAction::None)
    }

    fn name(&self) -> &str {
        "Settings"
    }
}

pub fn draw_settings_panel(
    f: &mut Frame,
    area: Rect,
    settings_state: &SettingsState,
    eq_state: &EqState,
    normalizer_state: &NormalizerState,
) {
    // Panel is active when rendered (router-based system)
    let is_active = true;
    draw_settings_list(f, area, settings_state, eq_state, normalizer_state, is_active);
}

fn draw_settings_list(
    f: &mut Frame,
    area: Rect,
    settings_state: &SettingsState,
    eq_state: &EqState,
    normalizer_state: &NormalizerState,
    is_active: bool,
) {
    let block = Block::default()
        .title(" Settings ")
        .borders(Borders::ALL)
        .border_style(if is_active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = settings_state
        .items
        .iter()
        .enumerate()
        .map(|(i, setting)| {
            let is_selected = settings_state.selected_index == i && !settings_state.is_dialog_open;

            let value_str = match setting {
                SettingsOption::Equalizer => {
                    if eq_state.eq_enabled {
                        "On"
                    } else {
                        "Off"
                    }
                }
                SettingsOption::Normalize => {
                    if normalizer_state.enabled {
                        "On"
                    } else {
                        "Off"
                    }
                },
            };

            let prefix = if is_selected { "▶ " } else { "  " };
            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!("{}{}", prefix, setting.label()), style),
                Span::raw(" "),
                Span::styled(format!("[{}]", value_str), Style::default().fg(Color::Cyan)),
            ]))
        })
        .collect();

    let list = List::new(items);
    f.render_widget(list, inner);
}
