use audido_core::modules::{self, core::CoreHandle};
use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
};

use crate::{
    router::{RouteAction, RouteHandler},
    state::AppState,
    states::AudioState,
};

// ==================================================================
// Playback Route Implementation
// ==================================================================

#[derive(Debug, Clone)]
pub struct PlaybackRoute;

impl RouteHandler for PlaybackRoute {
    fn render(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        draw_playback_panel(frame, area, &state.audio);
    }

    fn handle_input(
        &mut self,
        key: KeyCode,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> anyhow::Result<RouteAction> {
        let ctx = handle.ctx();
        match key {
            KeyCode::Up => {
                state.audio.volume = (state.audio.volume + 0.1).min(1.0);
                modules::playback::set_volume(ctx, state.audio.volume);
            }
            KeyCode::Down => {
                state.audio.volume = (state.audio.volume - 0.1).max(0.0);
                modules::playback::set_volume(ctx, state.audio.volume);
            }
            KeyCode::Right => {
                let new_pos = state.audio.position + 5.0;
                modules::playback::seek(ctx, new_pos);
            }
            KeyCode::Left => {
                let new_pos = (state.audio.position - 5.0).max(0.0);
                modules::playback::seek(ctx, new_pos);
            }
            KeyCode::Char(' ') => {
                if state.audio.is_playing {
                    handle.spawn(modules::playback::pause(handle.ctx()));
                } else {
                    handle.spawn(modules::playback::play(handle.ctx()));
                }
            }
            KeyCode::Char('s') => {
                modules::playback::stop(ctx);
            }
            KeyCode::Char('n') => {
                modules::queue::next(ctx);
            }
            KeyCode::Char('p') => {
                modules::queue::previous(ctx);
            }
            KeyCode::Char('l') => {
                let next_mode = state.next_loop_mode();
                modules::queue::set_loop_mode(ctx, next_mode);
            }
            _ => {}
        }
        Ok(RouteAction::None)
    }

    fn name(&self) -> &str {
        "Playback"
    }
}

/// Draw the playback panel
pub fn draw_playback_panel(f: &mut Frame, area: Rect, audio_state: &AudioState) {
    // Panel is active when rendered (router-based system)
    let is_active = true;

    // let has_cover = audio_state
    //     .metadata
    //     .as_ref()
    //     .map_or(false, |m| m.cover.is_some());

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(16), // Now playing info
            Constraint::Length(3), // Progress bar
            Constraint::Length(3), // Controls info
            Constraint::Min(0),    // Status/spacer
        ])
        .split(area);

    draw_now_playing(f, chunks[0], audio_state, is_active);
    draw_progress(f, chunks[1], audio_state);
}

/// Draw the now playing section
fn draw_now_playing(f: &mut Frame, area: Rect, audio_state: &AudioState, is_active: bool) {
    let border_style = if is_active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" 🎵 Now Playing ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(ref metadata) = audio_state.metadata {
        let title = metadata.title.as_deref().unwrap_or("Unknown Title");
        let artist = metadata.author.as_deref().unwrap_or("Unknown Artist");
        let album = metadata.album.as_deref().unwrap_or("Unknown Album");

        let text = vec![
            Line::from(vec![Span::styled(
                title,
                Style::default().fg(Color::White).bold(),
            )]),
            Line::from(vec![Span::styled(artist, Style::default().fg(Color::Gray))]),
            Line::from(vec![Span::styled(
                album,
                Style::default().fg(Color::DarkGray),
            )]),
        ];

        let paragraph = Paragraph::new(text);
        if let Some(protocol) = audio_state.cover_image_protocol.get() {
            let inner_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(30), // Fixed width for the cover art
                    Constraint::Length(1), // Margin
                    Constraint::Min(0),     // Remaining width for the text
                ])
                .split(inner);
            
            let image_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray));

            let image_area = image_block.inner(inner_chunks[0]);
            
            f.render_widget(image_block, inner_chunks[0]);
            let image_widget = ratatui_image::Image::new(protocol);
            f.render_widget(image_widget, image_area);
            
            f.render_widget(paragraph, inner_chunks[2]);
        } else {
            f.render_widget(paragraph, inner);
        }
    } else {
        let text = Paragraph::new("No audio loaded").style(Style::default().fg(Color::DarkGray));
        f.render_widget(text, inner);
    }
}

/// Draw the progress bar
fn draw_progress(f: &mut Frame, area: Rect, audio_state: &AudioState) {
    let progress_pct = (audio_state.progress() * 100.0) as u16;
    let position_str = AudioState::format_time(audio_state.position);
    let duration_str = AudioState::format_time(audio_state.duration);

    let label = format!("{} / {}", position_str, duration_str);

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL))
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
        .percent(progress_pct)
        .label(label);

    f.render_widget(gauge, area);
}
