use std::collections::BTreeMap;

use audido_core::modules::{self, core::CoreHandle};
use ratatui::{
    Frame,
    buffer::Buffer,
    crossterm::event::KeyCode,
    layout::{Direction, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, StatefulWidget},
};
use ratatui_hypertile::{
    Hypertile, HypertileAction, HypertileWidget, PaneId, PaneSnapshot, Towards,
};

use crate::{
    router::{RouteAction, RouteHandler}, routes::playback::PlaybackRoute, state::AppState, states::{BrowserFileDialog, browser::{ActiveBrowserPane, BrowserSource}}, ui::{DialogProperties, draw_generic_dialog},
};

/// Browser route - handles both browsing and file dialog as internal state
#[derive(Debug, Clone)]
pub struct BrowserRoute {
    layout: Hypertile,
    labels: BTreeMap<PaneId, String>,
}

impl BrowserRoute {
    pub fn new() -> Self {
        let mut layout = Hypertile::new();
        let mut labels = BTreeMap::new();

        labels.insert(PaneId::ROOT, "sources".to_string());
        if let Ok(content_id) = layout.split_focused(Direction::Horizontal) {
            labels.insert(content_id, "content".to_string());
        }

        Self { layout, labels }
    }

    pub fn draw_browser_panel(&mut self, f: &mut Frame, area: Rect, state: &AppState) {
        let labels = &self.labels;

        let hypertile_widget = HypertileWidget::new(|pane, buf| {
            let pane_label = labels.get(&pane.id).map(|s| s.as_str());
            draw_browser_panel_inside_hypertile(pane, buf, state, pane_label);
        });

        f.render_stateful_widget(hypertile_widget, area, &mut self.layout);
    }
}

impl RouteHandler for BrowserRoute {
    fn render(&mut self, frame: &mut Frame, area: Rect, state: &AppState) {
        self.draw_browser_panel(frame, area, state);

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
            // Dynamically resolve the active pane from the layout
            let active_pane = match self.layout.focused_pane().and_then(|id| self.labels.get(&id)).map(|s| s.as_str()) {
                Some("sources") => ActiveBrowserPane::Sources,
                _ => ActiveBrowserPane::Files, // Default to content area
            };

            match key {
                KeyCode::Up => state.browser.prev(active_pane.clone()),
                KeyCode::Down => state.browser.next(active_pane.clone()),

                KeyCode::Left => {
                    self.layout.apply_action(HypertileAction::FocusDirection {
                        direction: Direction::Horizontal,
                        towards: Towards::Start,
                    });
                }
                KeyCode::Right => {
                    self.layout.apply_action(HypertileAction::FocusDirection {
                        direction: Direction::Horizontal,
                        towards: Towards::End,
                    });
                }

                KeyCode::Enter => {
                    if active_pane == ActiveBrowserPane::Files {
                        if let Some(path) = state.browser.enter(active_pane) {
                            state.browser.open_dialog(path);
                        }
                    } else if active_pane == ActiveBrowserPane::Sources {
                        self.layout.apply_action(HypertileAction::FocusDirection {
                            direction: Direction::Horizontal,
                            towards: Towards::End,
                        });
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

fn draw_browser_panel_inside_hypertile(
    pane: PaneSnapshot,
    buf: &mut Buffer,
    state: &AppState,
    pane_label: Option<&str>,
) {
    let accent = state.theme.foreground_color;

    // Highlight the border of the currently focused pane
    let border_color = if pane.is_focused {
        accent
    } else {
        Color::DarkGray
    };

    let browser_state = &state.browser;

    if pane_label == Some("sources") {
        let block = Block::default()
            .title(" Sources ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let items: Vec<ListItem> = browser_state
            .sources
            .items
            .iter()
            .map(|source| {
                let text = match source {
                    BrowserSource::LocalFiles => "🖫 Local Files",
                    BrowserSource::YouTube => "🌐 YouTube",
                    BrowserSource::Playlists => "📝 Playlists",
                };
                ListItem::new(Line::from(vec![
                    Span::raw("   "),
                    Span::raw(text),
                ]))
            })
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_style(
                Style::default()
                    .fg(accent)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");

        let mut list_state = browser_state.sources.state.clone();
        StatefulWidget::render(list, pane.rect, buf, &mut list_state);
    } else {
        let title = if browser_state.current_dir.as_os_str().is_empty() {
            " Browser: System Drives ".to_string()
        } else {
            format!(" Browser: {} ", browser_state.current_dir.to_string_lossy())
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let items: Vec<ListItem> = browser_state
            .files
            .items
            .iter()
            .map(|item| {
                let icon = if item.is_dir { "📁" } else { "🎵" };
                let color = if item.is_dir {
                    Color::Blue
                } else {
                    Color::White
                };

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

        // We must clone the list state to pass it into the stateful widget renderer
        let mut list_state = browser_state.files.state.clone();
        StatefulWidget::render(list, pane.rect, buf, &mut list_state);
    }
}