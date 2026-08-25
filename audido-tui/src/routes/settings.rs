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
    states::SettingsOption,
    themes::AppTheme,
};

/// Settings route
#[derive(Debug, Clone)]
pub struct SettingsRoute;

impl RouteHandler for SettingsRoute {
    fn render(&mut self, frame: &mut Frame, area: Rect, state: &AppState) {
        draw_settings_panel(frame, area, state);
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
                let selected = state
                    .settings
                    .items
                    .get(state.settings.selected_index)
                    .copied();
                match selected {
                    Some(SettingsOption::Equalizer) => {
                        return Ok(RouteAction::Push(Box::new(EqualizerRoute::default())));
                    }
                    Some(SettingsOption::Theme) => {
                        // Cycle to the next theme
                        let theme = AppTheme::next_theme(state.theme.name);
                        state.set_theme(theme);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(RouteAction::None)
    }

    fn name(&self) -> &str {
        "Settings"
    }
}

pub fn draw_settings_panel(f: &mut Frame, area: Rect, state: &AppState) {
    draw_settings_list(f, area, state);
}

fn draw_settings_list(f: &mut Frame, area: Rect, state: &AppState) {
    let theme = &state.theme;
    let settings_state = &state.settings;
    let eq_state = &state.eq;
    let normalizer_state = &state.normalizer;

    let block = Block::default()
        .title(" Settings ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.foreground_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = settings_state
        .items
        .iter()
        .enumerate()
        .map(|(i, setting)| {
            let is_selected = settings_state.selected_index == i && !settings_state.is_dialog_open;

            let value_str: String = match setting {
                SettingsOption::Equalizer => {
                    if eq_state.eq_enabled {
                        "On".to_string()
                    } else {
                        "Off".to_string()
                    }
                }
                SettingsOption::Normalize => {
                    if normalizer_state.enabled {
                        "On".to_string()
                    } else {
                        "Off".to_string()
                    }
                }
                SettingsOption::Theme => theme.name.to_string(),
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
                Span::styled(
                    format!("[{}]", value_str),
                    Style::default().fg(theme.foreground_color),
                ),
            ]))
        })
        .collect();

    let list = List::new(items);
    f.render_widget(list, inner);
}
