use std::collections::BTreeMap;

use audido_core::modules::{self, core::CoreHandle};
use audido_core::{browser::FileEntry, modules::youtube::ytdlp::PlaylistEntry};
use ratatui::{
    Frame,
    buffer::Buffer,
    crossterm::event::KeyCode,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, StatefulWidget, Widget},
};
use ratatui_hypertile::{
    Hypertile, HypertileAction, HypertileWidget, PaneId, PaneSnapshot, Towards,
};

use crate::{
    router::{RouteAction, RouteHandler},
    routes::playback::PlaybackRoute,
    state::AppState,
    states::{
        BrowserFileDialog,
        browser::{ActiveBrowserPane, BrowserSource, YoutubeBrowserFocus},
    },
    ui::{DialogProperties, draw_generic_dialog},
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

    fn active_pane(&self) -> ActiveBrowserPane {
        match self
            .layout
            .focused_pane()
            .and_then(|id| self.labels.get(&id))
            .map(String::as_str)
        {
            Some("sources") => ActiveBrowserPane::Sources,
            _ => ActiveBrowserPane::Entries,
        }
    }

    fn youtube_content_is_active(&self, state: &AppState) -> bool {
        !state.browser.is_dialog_open()
            && self.active_pane() == ActiveBrowserPane::Entries
            && state.browser.current_source() == &BrowserSource::YouTube
    }
}

impl RouteHandler for BrowserRoute {
    fn render(&mut self, frame: &mut Frame, area: Rect, state: &AppState) {
        self.draw_browser_panel(frame, area, state);

        if let BrowserFileDialog::Open {
            title, selected, ..
        } = &state.browser.dialog
        {
            let options = vec!["Play Now", "Add to Queue"];

            let props = DialogProperties {
                title,
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
                    if let BrowserFileDialog::Open {
                        source, selected, ..
                    } = &state.browser.dialog
                    {
                        let source = source.clone();
                        let selected = *selected;
                        let ctx = handle.ctx();

                        if selected == 0 {
                            handle.spawn(modules::queue::play_source_immediately(ctx, source));
                            state.browser.close_dialog();
                            return Ok(RouteAction::Replace(Box::new(PlaybackRoute)));
                        } else {
                            let tokio_handle = ctx.tokio_handle.clone();
                            tokio_handle.spawn(async move {
                                modules::queue::add_sources_to_queue(ctx, vec![source]).await;
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
            let active_pane = self.active_pane();

            if self.youtube_content_is_active(state) {
                match key {
                    KeyCode::Char(character) => {
                        state.browser.youtube_focus = YoutubeBrowserFocus::Search;
                        state.browser.search_query.push(character);
                        return Ok(RouteAction::None);
                    }
                    KeyCode::Backspace => {
                        state.browser.youtube_focus = YoutubeBrowserFocus::Search;
                        state.browser.search_query.pop();
                        return Ok(RouteAction::None);
                    }
                    KeyCode::Delete => {
                        state.browser.youtube_focus = YoutubeBrowserFocus::Search;
                        state.browser.search_query.clear();
                        return Ok(RouteAction::None);
                    }
                    KeyCode::Enter => {
                        match state.browser.youtube_focus {
                            YoutubeBrowserFocus::Search => {
                                state.browser.search_youtube(handle.ctx());
                            }
                            YoutubeBrowserFocus::Entries => {
                                state.browser.open_selected_youtube_dialog();
                            }
                        }
                        return Ok(RouteAction::None);
                    }
                    KeyCode::Down => {
                        if state.browser.youtube_focus == YoutubeBrowserFocus::Search {
                            if !state.browser.entries.items.is_empty() {
                                state.browser.youtube_focus = YoutubeBrowserFocus::Entries;
                            }
                        } else {
                            state.browser.next(ActiveBrowserPane::Entries);
                        }
                        return Ok(RouteAction::None);
                    }
                    KeyCode::Up => {
                        if state.browser.youtube_focus == YoutubeBrowserFocus::Entries {
                            state.browser.prev(ActiveBrowserPane::Entries);
                        }
                        return Ok(RouteAction::None);
                    }
                    KeyCode::Esc => {
                        if state.browser.youtube_focus == YoutubeBrowserFocus::Entries {
                            state.browser.youtube_focus = YoutubeBrowserFocus::Search;
                            return Ok(RouteAction::None);
                        }
                    }
                    KeyCode::PageDown => {
                        state.browser.next_youtube_page(handle.ctx());
                        return Ok(RouteAction::None);
                    }
                    KeyCode::PageUp => {
                        state.browser.previous_youtube_page(handle.ctx());
                        return Ok(RouteAction::None);
                    }
                    _ => {}
                }
            }

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
                    if active_pane == ActiveBrowserPane::Entries
                        && state.browser.current_source() == &BrowserSource::LocalFiles
                    {
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

    fn intercept_global_key(
        &mut self,
        key: KeyCode,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> crate::router::InterceptKeyResult {
        if state.browser.is_dialog_open() {
            let _ = self.handle_input(key, state, handle);
            return crate::router::InterceptKeyResult::Handled;
        }

        if self.youtube_content_is_active(state)
            && matches!(
                key,
                KeyCode::Char(_)
                    | KeyCode::Backspace
                    | KeyCode::Delete
                    | KeyCode::Enter
                    | KeyCode::PageDown
                    | KeyCode::PageUp
                    | KeyCode::Esc
            )
        {
            let _ = self.handle_input(key, state, handle);
            crate::router::InterceptKeyResult::Handled
        } else {
            crate::router::InterceptKeyResult::Ignored
        }
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
        draw_source_panel(state, pane.rect, buf, accent, border_color);
    } else {
        match browser_state.current_source() {
            BrowserSource::LocalFiles => {
                draw_local_browser_panel(state, pane.rect, buf, border_color)
            }
            BrowserSource::YouTube => {
                draw_youtube_browser_panel(state, pane.rect, buf, border_color)
            }
            BrowserSource::Playlists => draw_empty_provider_panel(
                " Playlists ",
                "Playlist browsing is not implemented yet",
                pane.rect,
                buf,
                border_color,
            ),
        }
    }
}

fn draw_source_panel(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    accent: Color,
    border_color: Color,
) {
    let browser_state = &state.browser;
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
            ListItem::new(Line::from(vec![Span::raw("   "), Span::raw(text)]))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().fg(accent).add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    let mut list_state = browser_state.sources.state.clone();
    StatefulWidget::render(list, area, buf, &mut list_state);
}

fn draw_local_browser_panel(state: &AppState, area: Rect, buf: &mut Buffer, border_color: Color) {
    let browser_state = &state.browser;
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
        .entries
        .items
        .iter()
        .filter_map(|entry| {
            let item = entry.downcast_ref::<FileEntry>()?;
            let icon = if item.is_dir { "📁" } else { "🎵" };
            let color = if item.is_dir {
                Color::Blue
            } else {
                Color::White
            };

            Some(ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", icon), Style::default().fg(color)),
                Span::raw(&item.name),
            ])))
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
    let mut list_state = browser_state.entries.state.clone();
    StatefulWidget::render(list, area, buf, &mut list_state);
}

fn draw_youtube_browser_panel(state: &AppState, area: Rect, buf: &mut Buffer, border_color: Color) {
    let browser_state = &state.browser;
    let block = Block::default()
        .title(" YouTube ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    Widget::render(block, area, buf);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(2),
            Constraint::Length(1),
        ])
        .split(inner);

    let search_border = if browser_state.youtube_focus == YoutubeBrowserFocus::Search {
        state.theme.foreground_color
    } else {
        Color::DarkGray
    };
    let search = Paragraph::new(browser_state.search_query.as_str()).block(
        Block::default()
            .title(" Search (Enter) ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(search_border)),
    );
    Widget::render(search, chunks[0], buf);

    let results_border = if browser_state.youtube_focus == YoutubeBrowserFocus::Entries {
        state.theme.foreground_color
    } else {
        Color::DarkGray
    };
    let result_block = Block::default()
        .title(" Search Results ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(results_border));
    if browser_state.is_searching {
        Widget::render(
            Paragraph::new("Searching YouTube…").block(result_block),
            chunks[1],
            buf,
        );
    } else if let Some(error) = &browser_state.search_error {
        Widget::render(
            Paragraph::new(error.as_str())
                .style(Style::default().fg(Color::Red))
                .block(result_block),
            chunks[1],
            buf,
        );
    } else {
        let items: Vec<ListItem> = browser_state
            .entries
            .items
            .iter()
            .filter_map(|entry| {
                let entry = entry.downcast_ref::<PlaylistEntry>()?;
                let uploader = entry.uploader.as_deref().unwrap_or("Unknown channel");
                let duration = format_duration(entry.duration);
                Some(ListItem::new(Line::from(vec![
                    Span::styled("▶ ", Style::default().fg(Color::Red)),
                    Span::raw(entry.title.as_str()),
                    Span::styled(
                        format!(" — {uploader} [{duration}]"),
                        Style::default().fg(Color::DarkGray),
                    ),
                ])))
            })
            .collect();

        if items.is_empty() {
            let message = if browser_state.submitted_query.is_some() {
                "No results on this page"
            } else {
                "Find your favorite music: type a query above and press Enter"
            };
            Widget::render(Paragraph::new(message).block(result_block), chunks[1], buf);
        } else {
            let list = List::new(items)
                .block(result_block)
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(">> ");
            let mut list_state = browser_state.entries.state.clone();
            StatefulWidget::render(list, chunks[1], buf, &mut list_state);
        }
    }

    let previous = if browser_state.page_idx > 0 {
        "PgUp: Previous"
    } else {
        ""
    };
    let next = if browser_state.has_next_page {
        "PgDn: Next"
    } else {
        ""
    };
    Widget::render(
        Paragraph::new(format!(
            "{previous:<16} Page {} {next:>16}  Enter: Actions  Esc: Search",
            browser_state.page_idx + 1
        )),
        chunks[2],
        buf,
    );
}

fn draw_empty_provider_panel(
    title: &str,
    message: &str,
    area: Rect,
    buf: &mut Buffer,
    border_color: Color,
) {
    Widget::render(
        Paragraph::new(message).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        ),
        area,
        buf,
    );
}

fn format_duration(duration: Option<f64>) -> String {
    let seconds = duration.unwrap_or_default().max(0.0).round() as u64;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}
