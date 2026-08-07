use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::state::AppState;
use crate::states::{AudioState, QueueState};

// pub Struct 

/// Draw the TUI interface
pub fn draw(f: &mut Frame, state: &AppState, router: &crate::router::Router) {
        // Main vertical split: Top Navigation (top) and Main Content (bottom)
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Top Navigation bar (1 line text + 2 borders)
            Constraint::Min(0),    // Main content area
        ])
        .split(f.area());

    draw_navigation_bar(f, main_chunks[0], state, router);
    draw_main_content(f, main_chunks[1], state, router);
}

/// Draw navigation menu vertically on the top
fn draw_navigation_bar(f: &mut Frame, area: Rect, _state: &AppState, router: &crate::router::Router) {
    let block = Block::default()
    .title(" Navigation ")
    .borders(Borders::ALL)
    .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let current_route_name = router.current().name();
    let tab_names = crate::router::tab_names();
    let mut nav_spans = Vec::new();

    for (i, tab_name) in tab_names.iter().enumerate() {
        let is_active = *tab_name == current_route_name;
        let prefix = if is_active { "▶ " } else { "  " };
        let style = if is_active {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        
        nav_spans.push(Span::styled(format!("{}{}", prefix, tab_name), style));
        
        // Add a separator between tabs
        if i < tab_names.len() - 1 {
            nav_spans.push(Span::raw("  |  "));
        }
    }

    // Create a single Line from the spans to display horizontally
    let paragraph = Paragraph::new(Line::from(nav_spans))
        .alignment(Alignment::Center);
        
    f.render_widget(paragraph, inner);
}

/// Draw the main content area based on active route
fn draw_main_content(f: &mut Frame, area: Rect, state: &AppState, router: &crate::router::Router) {
    // Split the main area into Content (top) and Footer (bottom)
    // Footer contains Controls (3 lines) and Status (3 lines)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // Panel specific content
            Constraint::Length(3), // Controls info
            Constraint::Length(3), // Status bar
        ])
        .split(area);

    let content_area = chunks[0];
    let controls_area = chunks[1];
    let status_area = chunks[2];

    // Draw the specific panel via the router
    router.current().render(f, content_area, state);

    // Draw global footers on every tab
    draw_controls(f, controls_area, state, router);
    draw_status(f, status_area, &state.audio, &state.queue);
}

/// Draw the controls help section
fn draw_controls(f: &mut Frame, area: Rect, _state: &AppState, router: &crate::router::Router) {
    let route_name = router.current().name();
    let controls = match route_name {
        "Playback" => {
            vec![
                Span::styled("[Space]", Style::default().fg(Color::Yellow)),
                Span::raw(" Play/Pause  "),
                Span::styled("[N/P]", Style::default().fg(Color::Yellow)),
                Span::raw(" Next/Prev  "),
                Span::styled("[L]", Style::default().fg(Color::Yellow)),
                Span::raw(" Loop  "),
                Span::styled("[←/→]", Style::default().fg(Color::Yellow)),
                Span::raw(" Seek  "),
                Span::styled("[Tab]", Style::default().fg(Color::Magenta)),
                Span::raw(" Switch Tab  "),
                Span::styled("[Q]", Style::default().fg(Color::Red)),
                Span::raw(" Quit"),
            ]
        }
        "Queue" => {
            vec![
                Span::styled("[↑/↓]", Style::default().fg(Color::Yellow)),
                Span::raw(" Navigate  "),
                Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
                Span::raw(" Play  "),
                Span::styled("[N/P]", Style::default().fg(Color::Yellow)),
                Span::raw(" Next/Prev  "),
                Span::styled("[L]", Style::default().fg(Color::Yellow)),
                Span::raw(" Loop  "),
                Span::styled("[Tab]", Style::default().fg(Color::Magenta)),
                Span::raw(" Switch Tab  "),
                Span::styled("[Q]", Style::default().fg(Color::Red)),
                Span::raw(" Quit"),
            ]
        }
        "Log" => {
            vec![
                Span::styled("[↑/↓]", Style::default().fg(Color::Yellow)),
                Span::raw(" Scroll  "),
                Span::styled("[Tab]", Style::default().fg(Color::Magenta)),
                Span::raw(" Switch Tab  "),
                Span::styled("[Q]", Style::default().fg(Color::Red)),
                Span::raw(" Quit"),
            ]
        }
        "Browser" | "File Options" => {
            vec![
                Span::styled("[↑/↓]", Style::default().fg(Color::Yellow)),
                Span::raw(" Nav  "),
                Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
                Span::raw(" Select  "),
                Span::styled("[Tab]", Style::default().fg(Color::Magenta)),
                Span::raw(" Switch Tab  "),
                Span::styled("[Q]", Style::default().fg(Color::Red)),
                Span::raw(" Quit"),
            ]
        }
        "Settings" => {
            vec![
                Span::styled("[↑/↓]", Style::default().fg(Color::Yellow)),
                Span::raw(" Navigate  "),
                Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
                Span::raw(" Select  "),
                Span::styled("[Tab]", Style::default().fg(Color::Magenta)),
                Span::raw(" Switch Tab  "),
                Span::styled("[Q]", Style::default().fg(Color::Red)),
                Span::raw(" Quit"),
            ]
        }
        "Equalizer" => {
            vec![
                Span::styled("[←/→]", Style::default().fg(Color::Yellow)),
                Span::raw(" Focus  "),
                Span::styled("[T]", Style::default().fg(Color::Yellow)),
                Span::raw(" Toggle  "),
                Span::styled("[M]", Style::default().fg(Color::Yellow)),
                Span::raw(" Mode  "),
                Span::styled("[A]", Style::default().fg(Color::Yellow)),
                Span::raw(" Add  "),
                Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
                Span::raw(" Back  "),
                Span::styled("[Q]", Style::default().fg(Color::Red)),
                Span::raw(" Quit"),
            ]
        }
        _ => {
            vec![
                Span::styled("[Tab]", Style::default().fg(Color::Magenta)),
                Span::raw(" Switch Tab  "),
                Span::styled("[Q]", Style::default().fg(Color::Red)),
                Span::raw(" Quit"),
            ]
        }
    };

    let paragraph = Paragraph::new(Line::from(controls))
        .block(Block::default().borders(Borders::ALL).title(" Controls "));

    f.render_widget(paragraph, area);
}

/// Draw the status section
fn draw_status(f: &mut Frame, area: Rect, audio: &AudioState, queue: &QueueState) {
    let status_style = if audio.error_message.is_some() {
        Style::default().fg(Color::Red)
    } else if audio.is_playing {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Yellow)
    };

    let loop_icon = queue.loop_mode.to_string();

    let volume_bar = format!("Vol: {:3.0}%", audio.volume * 100.0);
    let queue_info = format!("Queue: {}", queue.queue.len());
    let status_text = format!(
        "{}  |  {}  |  {}  |  {}",
        audio.status_message, volume_bar, queue_info, loop_icon
    );

    let paragraph = Paragraph::new(status_text)
        .style(status_style)
        .block(Block::default().borders(Borders::ALL).title(" Status "));

    f.render_widget(paragraph, area);
}

pub struct DialogProperties<'a> {
    // dialog title
    pub title: &'a str,
    // list of options and its callbacks
    pub options: Vec<&'a str>,
    pub selected_index: usize,
}

/// Draw a generic dialog with given properties
pub fn draw_generic_dialog(f: &mut Frame, area: Rect, props: DialogProperties) {
    // We make the height dynamic based on options count, with a minimum
    let height = (props.options.len() as u16) + 4;
    let width = 40;

    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    // Clear the area behind the dialog (so it looks like an overlay)
    f.render_widget(Clear, dialog_area);

    // Create the Block
    let block = Block::default()
        .title(format!(" {} ", props.title))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner_area = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    // Render the Options
    // We map the raw strings into styled Lines based on the selected_index
    let text: Vec<Line> = props
        .options
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let is_selected = i == props.selected_index;

            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            let prefix = if is_selected { "▶ " } else { "  " };

            Line::from(Span::styled(format!("{}{}", prefix, label), style))
        })
        .collect();

    let paragraph = Paragraph::new(text);
    f.render_widget(paragraph, inner_area);
}

/// Open a modal with custom content
pub fn open_modal<T>(
    f: &mut Frame,
    area: Rect,
    state: T,
    content: impl FnOnce(&mut Frame, Rect, T),
) {
    // We create a centered area for the modal
    let width = area.width.saturating_sub(20).min(60);
    let height = area.height.saturating_sub(10).min(20);
    let x = area.x + (area.width - width) / 2;
    let y = area.y + (area.height - height) / 2;
    let modal_area = Rect::new(x, y, width, height);

    // Clear the background behind the modal
    f.render_widget(Clear, modal_area);

    // Render the provided content in the modal area
    content(f, modal_area, state);
}
