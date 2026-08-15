use std::sync::OnceLock;

use ratatui::style::Color;

use crate::themes::{AppTheme, CoverArt, utils::image_to_ascii_paragraph};

impl AppTheme {
    pub fn hatsune_miku() -> Self {
        static THEME: OnceLock<AppTheme> = OnceLock::new();
        THEME.get_or_init(|| {
            // Include bytes relative to the cargo manifest directory (audido-tui)
            let bytes = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/images/hatsune_miku.png"));
            let img = image::load_from_memory(bytes).expect("Failed to load embedded Miku image");
            
            // Constrain ASCII size to 50 width and 14 height to match the UI block
            let ascii_art = image_to_ascii_paragraph(&img, 30, 14);

            Self {
                name: "Hatsune Miku",
                foreground_color: Color::Rgb(57, 197, 187),
                font_color: Color::Rgb(57, 197, 187),
                default_cover: CoverArt::AsciiArt(ascii_art),
            }
        }).clone()
    }

    pub fn kasane_teto() -> Self {
        static THEME: OnceLock<AppTheme> = OnceLock::new();
        THEME.get_or_init(|| {
            let bytes = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/images/kasane_teto.png"));
            let img = image::load_from_memory(bytes).expect("Failed to load embedded Teto image");
            let ascii_art = image_to_ascii_paragraph(&img, 30, 14);

            Self {
                name: "Kasane Teto",
                foreground_color: Color::Rgb(212, 66, 114),
                font_color: Color::Rgb(212, 66, 114),
                default_cover: CoverArt::AsciiArt(ascii_art),
            }
        }).clone()
    }

    pub fn default_theme() -> Self {
        Self {
            name: "Default",
            foreground_color: Color::Cyan,
            font_color: Color::Cyan,
            default_cover: CoverArt::None,
        }
    }

    /// Returns all available themes in order (used by the settings theme picker).
    pub fn all_themes() -> Vec<AppTheme> {
        vec![
            AppTheme::default_theme(),
            AppTheme::hatsune_miku(),
            AppTheme::kasane_teto(),
        ]
    }

    /// Returns the index of the current theme inside `all_themes()`, or 0 if not found.
    #[allow(dead_code)]
    pub fn current_index(name: &str) -> usize {
        Self::all_themes()
            .iter()
            .position(|t| t.name == name)
            .unwrap_or(0)
    }

    /// Returns the next theme after the one with the given name, cycling around.
    pub fn next_theme(current_name: &str) -> AppTheme {
        let themes = Self::all_themes();
        let idx = themes
            .iter()
            .position(|t| t.name == current_name)
            .unwrap_or(0);
        let next_idx = (idx + 1) % themes.len();
        themes.into_iter().nth(next_idx).unwrap()
    }
}
