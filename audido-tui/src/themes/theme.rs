use ratatui::{style::Color, text::Line};
use ratatui_image::protocol::Protocol;

/// The visual theme of the application.
#[derive(Clone)]
pub struct AppTheme {
    /// Human-readable theme name shown in settings
    pub name: &'static str,
    /// Primary foreground / accent color (borders, highlights, gauges)
    pub foreground_color: Color,
    /// Text / font color
    pub font_color: Color,
    /// Default cover art shown when a track has no embedded artwork
    pub default_cover: CoverArt,
}

/// The cover art to show in the Now Playing panel.
#[derive(Clone)]
pub enum CoverArt {
    /// An image rendered via a ratatui-image protocol
    Image(Protocol),
    /// ASCII art string
    AsciiArt(Vec<Line<'static>>),
    /// No cover art
    None,
}

