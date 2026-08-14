use audido_core::modules::{self, core::CoreHandle};
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
    routes::playback::PlaybackRoute,
    state::AppState,
    states::{BrowserFileDialog, BrowserState},
    ui::{DialogProperties, draw_generic_dialog},
};

/// Browser route - handles both browsing and file dialog as internal state
#[derive(Debug, Clone)]
pub struct BrowserRoute;

impl RouteHandler for BrowserRoute {
    fn render(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        draw_browser_panel(frame, area, &state.browser);

        if let BrowserFileDialog::Open { path, selected } = &state.browser.dialog {
            let filename = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown File".to_string());

            let options = vec!["Play Now", "Add to Queue"];

            let props = DialogProperties {
                title: &filename,
                options,
                selected_index: *selected,
            };

            draw_generic_dialog(frame, area, props);
        }
    }

    fn handle_input(
        &mut self,
        key: KeyCode,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> anyhow::Result<RouteAction> {
        // Check if dialog is open - handle dialog input
        if state.browser.is_dialog_open() {
            match key {
                KeyCode::Up | KeyCode::Down => {
                    state.browser.dialog_toggle();
                }
                KeyCode::Enter => {
                    if let BrowserFileDialog::Open { path, selected } = &state.browser.dialog {
                        let path_str = path.to_string_lossy().to_string();
                        let ctx = handle.ctx();

                        if *selected == 0 {
                            // Play Now — clear queue, add file, auto-play
                            let ctx_clone = ctx.clone();
                            let path_clone = path_str.clone();
                            handle.spawn(modules::queue::play_immediately(ctx_clone, path_clone));
                            state.browser.close_dialog();
                            return Ok(RouteAction::Replace(Box::new(PlaybackRoute)));
                        } else {
                            // Add to Queue only
                            let ctx_clone = ctx.clone();
                            ctx.tokio_handle.spawn(async move {
                                modules::queue::add_to_queue(ctx_clone, vec![path_str]).await;
                            });
                            state.browser.close_dialog();
                        }
                    }
                }
                KeyCode::Esc => {
                    state.browser.close_dialog();
                }
                _ => {}
            }
        } else {
            // Normal browser navigation
            match key {
                KeyCode::Up => state.browser.prev(),
                KeyCode::Down => state.browser.next(),
                KeyCode::Enter => {
                    if let Some(path) = state.browser.enter() {
                        state.browser.open_dialog(path);
                    }
                }
                _ => {}
            }
        }
        Ok(RouteAction::None)
    }

    fn name(&self) -> &str {
        "Browser"
    }
}

pub fn draw_browser_panel(f: &mut Frame, area: Rect, browser_state: &BrowserState) {
    let is_active = true;

    let title = if browser_state.current_dir.as_os_str().is_empty() {
        " Browser: System Drives ".to_string()
    } else {
        format!(" Browser: {} ", browser_state.current_dir.to_string_lossy())
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(if is_active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        });

    let items: Vec<ListItem> = browser_state
        .items
        .iter()
        .map(|item| {
            let icon = if item.is_dir { "📁" } else { "🎵" };
            let color = if item.is_dir { Color::Blue } else { Color::White };

            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", icon), Style::default().fg(color)),
                Span::raw(&item.name),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    let mut list_state = browser_state.list_state;
    f.render_stateful_widget(list, area, &mut list_state);
}
