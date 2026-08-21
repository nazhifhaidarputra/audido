use std::sync::Arc;

use audido_core::modules::{self, core::{CoreContext, CoreHandle}};
use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Bar, BarChart, BarGroup, Block, Borders, Gauge, Paragraph},
};

use crate::{
    router::{RouteAction, RouteHandler},
    state::AppState,
    states::AudioState,
    themes::CoverArt,
};

// ==================================================================
// Playback Route Implementation
// ==================================================================

#[derive(Debug, Clone)]
pub struct PlaybackRoute;

impl RouteHandler for PlaybackRoute {
    fn render(&mut self, frame: &mut Frame, area: Rect, state: &AppState) {
        draw_playback_panel(frame, area, state);
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
            KeyCode::Char('1') => seek_to_pct(ctx.clone(), state, 0.1),
            KeyCode::Char('2') => seek_to_pct(ctx.clone(), state, 0.2),
            KeyCode::Char('3') => seek_to_pct(ctx.clone(), state, 0.3),
            KeyCode::Char('4') => seek_to_pct(ctx.clone(), state, 0.4),
            KeyCode::Char('5') => seek_to_pct(ctx.clone(), state, 0.5),
            KeyCode::Char('6') => seek_to_pct(ctx.clone(), state, 0.6),
            KeyCode::Char('7') => seek_to_pct(ctx.clone(), state, 0.7),
            KeyCode::Char('8') => seek_to_pct(ctx.clone(), state, 0.8),
            KeyCode::Char('9') => seek_to_pct(ctx.clone(), state, 0.9),
            _ => {}
        }
        Ok(RouteAction::None)
    }

    fn name(&self) -> &str {
        "Playback"
    }
}

#[inline(always)]
fn seek_to_pct(ctx: Arc<CoreContext>, state: &AppState, pct: f32) {
    if state.audio.duration > 0.0 {
        let new_pos = state.audio.duration * pct;
        modules::playback::seek(ctx, new_pos);
    }
}

/// Draw the playback panel
pub fn draw_playback_panel(f: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(16), // Now playing info
            Constraint::Min(8),     // Spectrum visualizer
            Constraint::Length(3),  // Progress bar
        ])
        .split(area);

    draw_now_playing(f, chunks[0], state);
    draw_freq_spectrum(f, chunks[1], state);
    draw_progress(f, chunks[2], &state.audio, state.theme.foreground_color);
}

/// Draw the now playing section
fn draw_now_playing(f: &mut Frame, area: Rect, state: &AppState) {
    let audio_state = &state.audio;
    let theme = &state.theme;

    let border_style = Style::default()
        .fg(theme.foreground_color)
        .add_modifier(Modifier::BOLD);

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
            // Track has an embedded cover image → render it
            let inner_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(30), // Fixed width for the cover art
                    Constraint::Length(1),  // Margin
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
            // No embedded cover — fall back to theme default_cover
            match &theme.default_cover {
                CoverArt::AsciiArt(art_lines) => {
                    let inner_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Length(30), // ASCII art panel
                            Constraint::Length(1),  // Margin
                            Constraint::Min(0),     // Metadata text
                        ])
                        .split(inner);

                    let art_block = Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(theme.foreground_color));

                    let art_inner = art_block.inner(inner_chunks[0]);
                    f.render_widget(art_block, inner_chunks[0]);

                    let ascii_paragraph = Paragraph::new(art_lines.clone());
                    f.render_widget(ascii_paragraph, art_inner);

                    f.render_widget(paragraph, inner_chunks[2]);
                }
                CoverArt::Image(protocol) => {
                    let inner_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Length(30),
                            Constraint::Length(1),
                            Constraint::Min(0),
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
                }
                CoverArt::None => {
                    f.render_widget(paragraph, inner);
                }
            }
        }
    } else {
        let text = Paragraph::new("No audio loaded").style(Style::default().fg(Color::DarkGray));
        f.render_widget(text, inner);
    }
}

/// Draw the progress bar
fn draw_progress(f: &mut Frame, area: Rect, audio_state: &AudioState, accent: Color) {
    let progress_pct = (audio_state.progress() * 100.0) as u16;
    let position_str = AudioState::format_time(audio_state.position);
    let duration_str = AudioState::format_time(audio_state.duration);

    let label = format!("{} / {}", position_str, duration_str);

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(accent)))
        .gauge_style(Style::default().fg(accent).bg(Color::DarkGray))
        .percent(progress_pct)
        .label(label);

    f.render_widget(gauge, area);
}

/// Draw bar spectrum for the audio visualizer.
/// Renders between the `now_playing` panel and the `progress` bar.
fn draw_freq_spectrum(f: &mut Frame, area: Rect, state: &AppState) {
    let accent = state.theme.foreground_color;

    let border_style = Style::default().fg(accent);
    let block = Block::default()
        .title(" 〰 Spectrum ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let bins = state.audio.visualizer_config.bins();

    // Flat floor until real audio data arrives.
    let all_silent = bins.iter().all(|&v| v <= -130.0);
    if bins.is_empty() || all_silent {
        let msg = if state.audio.is_playing {
            "Analyzing…"
        } else {
            "No audio"
        };
        let text = Paragraph::new(msg)
            .style(Style::default().fg(Color::DarkGray))
            .centered();
        f.render_widget(text, inner);
        return;
    }

    const DB_FLOOR: f32 = -70.0; // silence floor
    const DB_CEIL: f32 = -10.0; // practical clip level for music
    let db_range = DB_CEIL - DB_FLOOR;

    let bar_width: u16 = 1;
    let bar_gap: u16 = 0;
    let max_bars = (inner.width as usize / (bar_width + bar_gap) as usize).max(1);

    let bar_height = inner.height as u64;
    if bar_height == 0 {
        return;
    }

    // ---------------------------------------------------------------
    // Logarithmic Frequency Mapping
    // ---------------------------------------------------------------
    // Fetch sample rate to calculate the Nyquist limit
    let sample_rate = state
        .audio
        .metadata
        .as_ref()
        .map(|m| m.sample_rate)
        .unwrap_or(44100) as f32;
    let nyquist = sample_rate / 2.0;

    const MIN_FREQ: f32 = 20.0; // Lowest audible bass
    let max_freq: f32 = nyquist.min(20000.0); // Highest audible treble limit

    let log_min = MIN_FREQ.log2();
    let log_max = max_freq.log2();

    // Gradient: bass → cyan, low-mids → green, high-mids → amber, treble → magenta.
    let gradient: &[(f32, Color)] = &[
        (0.00, Color::Rgb(0, 210, 210)),
        (0.33, Color::Rgb(40, 200, 60)),
        (0.66, Color::Rgb(230, 170, 0)),
        (1.00, Color::Rgb(210, 50, 210)),
    ];

    let lerp_color = |t: f32| -> Color {
        let t = t.clamp(0.0, 1.0);
        let mut lo = gradient[0];
        let mut hi = gradient[gradient.len() - 1];
        for win in gradient.windows(2) {
            if t >= win[0].0 && t <= win[1].0 {
                lo = win[0];
                hi = win[1];
                break;
            }
        }
        let span = (hi.0 - lo.0).max(1e-6);
        let s = ((t - lo.0) / span).clamp(0.0, 1.0);
        let lerp_u8 = |a: u8, b: u8| -> u8 { (a as f32 + s * (b as f32 - a as f32)) as u8 };
        match (lo.1, hi.1) {
            (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) => {
                Color::Rgb(lerp_u8(r0, r1), lerp_u8(g0, g1), lerp_u8(b0, b1))
            }
            _ => lo.1,
        }
    };

    let bars: Vec<Bar> = (0..max_bars)
        .map(|i| {
            // 1. Calculate the logarithmic frequency bounds for this visual bar
            let f_start = (log_min + (i as f32 / max_bars as f32) * (log_max - log_min)).exp2();
            let f_end = (log_min + ((i + 1) as f32 / max_bars as f32) * (log_max - log_min)).exp2();

            // 2. Map the frequencies to indices in our linear FFT bins array
            let mut bin_start = ((f_start / nyquist) * bins.len() as f32) as usize;
            let mut bin_end = ((f_end / nyquist) * bins.len() as f32).ceil() as usize;

            // 3. Ensure bounds are safe and we grab at least 1 bin
            bin_start = bin_start.clamp(0, bins.len().saturating_sub(1));
            bin_end = bin_end.clamp(bin_start + 1, bins.len());

            let bin_slice = &bins[bin_start..bin_end];

            // 4. Extract the peak dB within this logarithmic chunk
            let peak_db = bin_slice.iter().copied().fold(f32::NEG_INFINITY, f32::max);

            let clamped = peak_db.clamp(DB_FLOOR, DB_CEIL);
            let norm = (clamped - DB_FLOOR) / db_range; // 0.0 – 1.0
            let height = (norm * bar_height as f32).round() as u64;

            let t = i as f32 / max_bars.max(1) as f32;
            let color = lerp_color(t);

            Bar::default()
                .value(height)
                .text_value(String::new())
                .style(Style::default().fg(color))
        })
        .collect();

    let bar_chart = BarChart::default()
        .data(BarGroup::default().bars(&bars))
        .bar_width(bar_width)
        .bar_gap(bar_gap)
        .max(bar_height);

    f.render_widget(bar_chart, inner);
}
