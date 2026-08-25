use audido_core::{
    dsp::normalization::NormalizationMode,
    modules::{self, core::CoreHandle},
};
use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::{
    router::{InterceptKeyResult, RouteAction, RouteHandler, get_next_tab, route_for_name},
    state::AppState,
    states::normalizer::NormalizerState,
};

const SETTING_COUNT: usize = 4;

#[derive(Debug, Clone, Default)]
pub struct NormalizationRoute {
    selected: usize,
}

impl NormalizationRoute {
    fn next(&mut self) {
        self.selected = (self.selected + 1) % SETTING_COUNT;
    }

    fn previous(&mut self) {
        self.selected = (self.selected + SETTING_COUNT - 1) % SETTING_COUNT;
    }

    fn toggle_enabled(state: &mut AppState, handle: &CoreHandle) {
        state.normalizer.toggle_enabled();
        modules::normalizer::set_enabled(handle.ctx(), state.normalizer.enabled);
    }

    fn toggle_mode(state: &mut AppState, handle: &CoreHandle) {
        state.normalizer.toggle_mode();
        modules::normalizer::set_mode(handle.ctx(), state.normalizer.mode);
    }

    fn adjust_selected(&self, state: &mut AppState, handle: &CoreHandle, increase: bool) {
        match self.selected {
            0 => Self::toggle_enabled(state, handle),
            1 => Self::toggle_mode(state, handle),
            2 => {
                state.normalizer.adjust_target(increase);
                modules::normalizer::set_target_level(
                    handle.ctx(),
                    state.normalizer.target_level(),
                );
            }
            3 if state.normalizer.mode == NormalizationMode::Loudness => {
                state.normalizer.adjust_headroom(increase);
                modules::normalizer::set_headroom(handle.ctx(), state.normalizer.headroom_db);
            }
            _ => {}
        }
    }
}

impl RouteHandler for NormalizationRoute {
    fn render(&mut self, frame: &mut Frame, area: Rect, state: &AppState) {
        draw_normalization_panel(
            frame,
            area,
            &state.normalizer,
            self.selected,
            state.theme.foreground_color,
        );
    }

    fn handle_input(
        &mut self,
        key: KeyCode,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> anyhow::Result<RouteAction> {
        match key {
            KeyCode::Up => self.previous(),
            KeyCode::Down => self.next(),
            KeyCode::Left => self.adjust_selected(state, handle, false),
            KeyCode::Right => self.adjust_selected(state, handle, true),
            KeyCode::Enter => match self.selected {
                0 => Self::toggle_enabled(state, handle),
                1 => Self::toggle_mode(state, handle),
                _ => {}
            },
            KeyCode::Char('t') => Self::toggle_enabled(state, handle),
            KeyCode::Char('m') => Self::toggle_mode(state, handle),
            _ => {}
        }
        Ok(RouteAction::None)
    }

    fn name(&self) -> &str {
        "Normalization"
    }

    fn intercept_global_key(
        &mut self,
        key: KeyCode,
        _state: &mut AppState,
        _handle: &CoreHandle,
    ) -> InterceptKeyResult {
        if key == KeyCode::Tab
            && let Some(next_tab_name) = get_next_tab("Settings")
        {
            return InterceptKeyResult::HandledAndNavigate(RouteAction::Reset(route_for_name(
                next_tab_name,
            )));
        }
        InterceptKeyResult::Ignored
    }
}

fn draw_normalization_panel(
    frame: &mut Frame,
    area: Rect,
    normalizer: &NormalizerState,
    selected: usize,
    accent: Color,
) {
    let enabled_style = if normalizer.enabled {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    };
    let block = Block::default()
        .title(Line::from(vec![
            Span::raw(" Normalization • "),
            Span::styled(if normalizer.enabled { "ON" } else { "OFF" }, enabled_style),
            Span::raw(" "),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(5)])
        .split(inner);

    let peak_ceiling = match normalizer.mode {
        NormalizationMode::Peak => format!(
            "{:+.1} dBFS (from target)",
            20.0 * normalizer.peak_target.log10()
        ),
        NormalizationMode::Loudness => format!("-{:.1} dBFS", normalizer.headroom_db),
    };
    let values = [
        if normalizer.enabled {
            "On".to_string()
        } else {
            "Off".to_string()
        },
        normalizer.mode.to_string(),
        normalizer.target_label(),
        peak_ceiling,
    ];
    let labels = ["Enabled", "Mode", "Target", "Sample-peak ceiling"];
    let items = labels
        .iter()
        .zip(values)
        .enumerate()
        .map(|(index, (label, value))| {
            let selected_style = if index == selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let value_style = if index == 3 && normalizer.mode == NormalizationMode::Peak {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(accent)
            };
            ListItem::new(Line::from(vec![
                Span::styled(if index == selected { "▶ " } else { "  " }, selected_style),
                Span::styled(format!("{label:<21}"), selected_style),
                Span::styled(value, value_style),
            ]))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );

    let meter_text = vec![
        Line::from(vec![
            Span::styled("Input: ", Style::default().fg(Color::Gray)),
            Span::styled(
                normalizer.measured_label(),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw("    "),
            Span::styled("Applied gain: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{:+.1} dB", normalizer.current_gain_db),
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(""),
        Line::styled(
            match normalizer.mode {
                NormalizationMode::Peak => {
                    "Peak mode follows the selected sample-peak target with slow gain recovery."
                }
                NormalizationMode::Loudness => {
                    "Loudness mode follows a 400 ms BS.1770 K-weighted measurement."
                }
            },
            Style::default().fg(Color::Gray),
        ),
        Line::styled(
            "The ceiling is sample-peak protection; it is not a true-peak (dBTP) limiter.",
            Style::default().fg(Color::DarkGray),
        ),
    ];
    frame.render_widget(
        Paragraph::new(meter_text).block(Block::default().title(" Live meter ")),
        chunks[1],
    );
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    #[test]
    fn loudness_settings_render_target_and_meter_units() {
        let mut normalizer = NormalizerState::new();
        normalizer.mode = NormalizationMode::Loudness;
        normalizer.measured_level = -20.5;
        let backend = TestBackend::new(90, 18);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                draw_normalization_panel(frame, frame.area(), &normalizer, 2, Color::Magenta);
            })
            .unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("-18.0 LUFS"));
        assert!(rendered.contains("-20.5 LUFS"));
    }
}
