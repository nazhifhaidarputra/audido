use image::{DynamicImage, GenericImageView};
use ratatui::{style::{Color, Style}, text::{Line, Span}};

/// Converts a DynamicImage into a vector of Ratatui Lines containing colored ASCII art.
pub fn image_to_ascii_paragraph<'a>(img: &DynamicImage, width: u16, height: u16) -> Vec<Line<'a>> {
    let resized = img.resize_exact(
        width as u32, 
        height as u32, 
        image::imageops::FilterType::Nearest
    );
    
    let ascii_chars = ['@', '%', '#', '*', '+', '=', '-', ':', '.', ' '];
    let mut lines = Vec::new();

    for y in 0..resized.height() {
        let mut spans = Vec::new();
        for x in 0..resized.width() {
            let pixel = resized.get_pixel(x, y);
            let r = pixel[0];
            let g = pixel[1];
            let b = pixel[2];
            let a = pixel[3];

            if a < 128 {
                spans.push(Span::raw(" "));
                continue;
            }

            let luminance = (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) as u8;

            let char_idx = (luminance as usize * (ascii_chars.len() - 1)) / 255;
            let ascii_char = ascii_chars[char_idx].to_string();

            spans.push(Span::styled(
                ascii_char,
                Style::default().fg(Color::Rgb(r, g, b)),
            ));
        }
        lines.push(Line::from(spans));
    }

    lines
}