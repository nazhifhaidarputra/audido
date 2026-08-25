use std::sync::OnceLock;

use ratatui::style::Color;

use crate::themes::{AppTheme, CoverArt, CoverArtRenderMode, ImageSource};

impl AppTheme {
    pub fn hatsune_miku() -> Self {
        static THEME: OnceLock<AppTheme> = OnceLock::new();
        THEME
            .get_or_init(|| {
                let source = ImageSource::Url(
                    "https://i.pinimg.com/736x/fc/23/21/fc2321ef283919ba216701482815a6c1.jpg",
                );

                Self::from_image_source(
                    "Hatsune Miku",
                    Color::Rgb(57, 197, 187),
                    Color::Rgb(57, 197, 187),
                    source,
                    CoverArtRenderMode::Ascii,
                )
            })
            .clone()
    }

    pub fn kasane_teto() -> Self {
        static THEME: OnceLock<AppTheme> = OnceLock::new();
        THEME
            .get_or_init(|| {
                let source = ImageSource::Url(
                    "https://i.pinimg.com/736x/b1/9a/07/b19a073ce8652e616624404b0e1c6b71.jpg"
                );

                Self::from_image_source(
                    "Kasane Teto",
                    Color::Rgb(212, 66, 114),
                    Color::Rgb(212, 66, 114),
                    source,
                    CoverArtRenderMode::Ascii,
                )
            })
            .clone()
    }

    /// Build a theme whose default cover can come from bytes, a file, a URL,
    /// or an already-decoded image.
    pub fn from_image_source(
        name: &'static str,
        foreground_color: Color,
        font_color: Color,
        source: ImageSource<'_>,
        render_mode: CoverArtRenderMode,
    ) -> Self {
        let default_cover = CoverArt::from_source(source, render_mode).unwrap_or_else(|error| {
            log::warn!("Unable to load cover for theme {name}: {error:#}");
            CoverArt::none()
        });

        Self {
            name,
            foreground_color,
            font_color,
            default_cover,
        }
    }

    pub fn default_theme() -> Self {
        Self {
            name: "Default",
            foreground_color: Color::Cyan,
            font_color: Color::Cyan,
            default_cover: CoverArt::none(),
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
            .position(|theme| theme.name == name)
            .unwrap_or(0)
    }

    /// Returns the next theme after the one with the given name, cycling around.
    pub fn next_theme(current_name: &str) -> AppTheme {
        let themes = Self::all_themes();
        let index = themes
            .iter()
            .position(|theme| theme.name == current_name)
            .unwrap_or(0);
        let next_index = (index + 1) % themes.len();
        themes.into_iter().nth(next_index).unwrap()
    }
}
