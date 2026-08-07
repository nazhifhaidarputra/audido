use std::cell::RefCell;

use audido_core::modules::core::CoreHandle;
use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::*,
    widgets::*,
};

use crate::{
    logger::{LOG_BUFFER, get_level_style},
    router::{RouteAction, RouteHandler},
    state::AppState,
};

// Log route with interior mutability for the ListState
#[derive(Debug)]
pub struct LogRoute {
    // RefCell allows mutation even when we only have &self in render()
    list_state: RefCell<ListState>,
    // Track if user is sticking to the bottom
    stick_to_bottom: bool,
}

impl LogRoute {
    pub fn new() -> Self {
        Self {
            list_state: RefCell::new(ListState::default()),
            stick_to_bottom: true,
        }
    }
}

impl RouteHandler for LogRoute {
    fn render(&self, frame: &mut Frame, area: Rect, _state: &AppState) {
        let buffer = LOG_BUFFER.lock().unwrap();

        let items: Vec<ListItem> = buffer
            .iter()
            .map(|record| {
                let level_style = get_level_style(record.level);

                let content = Line::from(vec![
                    Span::styled(
                        format!("{} ", record.timestamp),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(format!("[{:<5}] ", record.level), level_style),
                    Span::raw(&record.message),
                ]);

                ListItem::new(content)
            })
            .collect();

        let log_list = List::new(items)
            .block(
                Block::default()
                    .title(" 📋 Log (Up/Down to Scroll) ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        // Borrow the state mutably to pass it to the widget
        let mut state = self.list_state.borrow_mut();

        // Auto-scroll logic inside render:
        // If we are sticky, ensure we are selecting the newest item *before* drawing.
        // This ensures that if new logs come in (without key presses), we still scroll to them.
        if self.stick_to_bottom && !buffer.is_empty() {
            state.select(Some(buffer.len() - 1));
        }

        frame.render_stateful_widget(log_list, area, &mut *state);
    }

    fn handle_input(
        &mut self,
        key: KeyCode,
        _state: &mut AppState,
        __handle: &CoreHandle,
    ) -> anyhow::Result<RouteAction> {
        let buffer_len = LOG_BUFFER.lock().unwrap().len();
        if buffer_len == 0 {
            return Ok(RouteAction::None);
        }

        let mut state = self.list_state.borrow_mut();

        match key {
            KeyCode::Up => {
                let i = match state.selected() {
                    Some(i) => {
                        if i == 0 {
                            0
                        } else {
                            i - 1
                        }
                    }
                    None => buffer_len - 1,
                };
                state.select(Some(i));
                self.stick_to_bottom = false;
            }
            KeyCode::Down => {
                let i = match state.selected() {
                    Some(i) => {
                        if i >= buffer_len - 1 {
                            buffer_len - 1
                        } else {
                            i + 1
                        }
                    }
                    None => 0,
                };
                state.select(Some(i));
                // If we manually scrolled to the bottom, re-enable stickiness
                if i == buffer_len - 1 {
                    self.stick_to_bottom = true;
                }
            }
            KeyCode::End | KeyCode::PageDown => {
                state.select(Some(buffer_len - 1));
                self.stick_to_bottom = true;
            }
            KeyCode::Home | KeyCode::PageUp => {
                state.select(Some(0));
                self.stick_to_bottom = false;
            }
            _ => {}
        }

        Ok(RouteAction::None)
    }

    fn name(&self) -> &str {
        "Log"
    }
}
