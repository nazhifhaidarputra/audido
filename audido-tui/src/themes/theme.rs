use std::{io::Read, path::Path, time::Duration};

use anyhow::{Context, bail};
use image::DynamicImage;
use ratatui::{layout::Size, style::Color};
use ratatui_image::{Resize, picker::Picker, protocol::Protocol};

const COVER_SIZE: Size = Size::new(28, 12);
const MAX_REMOTE_IMAGE_BYTES: u64 = 25 * 1024 * 1024;

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

/// An input accepted by the theme image loader.
///
/// All variants are decoded into the same [`DynamicImage`] representation, so
/// callers can change a factory theme from a bundled image to a local file or
/// an internet URL without changing its rendering code.
pub enum ImageSource<'a> {
    /// Raw encoded image data, such as bytes returned by `include_bytes!` or a blob store.
    Bytes(&'a [u8]),
    /// A file on the local filesystem.
    File(&'a Path),
    /// An HTTP(S) or `file://` URL.
    Url(&'a str),
    /// An image already decoded by the caller.
    #[allow(dead_code)]
    Image(DynamicImage),
}

impl ImageSource<'_> {
    /// Decode this source using format detection provided by the `image` crate.
    pub fn load(self) -> anyhow::Result<DynamicImage> {
        match self {
            Self::Bytes(bytes) => {
                image::load_from_memory(bytes).context("failed to decode image bytes")
            }
            Self::File(path) => image::open(path)
                .with_context(|| format!("failed to open image file {}", path.display())),
            Self::Url(url) => load_image_url(url),
            Self::Image(image) => Ok(image),
        }
    }
}

/// The cover art to show in the Now Playing panel.
#[derive(Clone)]
pub struct CoverArt {
    /// Terminal-specific rendering protocol, initialized after terminal detection.
    pub protocol: Option<Protocol>,
    /// Source image shared by ASCII and terminal-image rendering modes.
    pub source_image: Option<DynamicImage>,
    pub render_mode: CoverArtRenderMode,
}

impl CoverArt {
    pub fn none() -> Self {
        Self {
            protocol: None,
            source_image: None,
            render_mode: CoverArtRenderMode::Ascii,
        }
    }

    /// Load cover art from any supported source.
    pub fn from_source(
        source: ImageSource<'_>,
        render_mode: CoverArtRenderMode,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            protocol: None,
            source_image: Some(source.load()?),
            render_mode,
        })
    }

    /// Load a path or URL using one configuration string.
    ///
    /// HTTP(S) and `file://` values are treated as URLs; all other values are
    /// interpreted as filesystem paths.
    #[allow(dead_code)]
    pub fn from_location(location: &str, render_mode: CoverArtRenderMode) -> anyhow::Result<Self> {
        let source = match reqwest::Url::parse(location) {
            Ok(url) if matches!(url.scheme(), "http" | "https" | "file") => {
                ImageSource::Url(location)
            }
            _ => ImageSource::File(Path::new(location)),
        };

        Self::from_source(source, render_mode)
    }

    /// Build terminal-specific state after the application's picker is detected.
    pub fn prepare(&mut self, picker: &Picker) -> anyhow::Result<()> {
        self.protocol = None;

        if self.render_mode == CoverArtRenderMode::NormalImage {
            let Some(image) = self.source_image.as_ref() else {
                return Ok(());
            };

            self.protocol = Some(
                picker
                    .new_protocol(image.clone(), COVER_SIZE, Resize::Fit(None))
                    .context("failed to prepare theme cover image for this terminal")?,
            );
        }

        Ok(())
    }
}

impl Default for CoverArt {
    fn default() -> Self {
        Self::none()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverArtRenderMode {
    Ascii,
    NormalImage,
}

fn load_image_url(url: &str) -> anyhow::Result<DynamicImage> {
    let parsed = reqwest::Url::parse(url).with_context(|| format!("invalid image URL: {url}"))?;

    if parsed.scheme() == "file" {
        let path = parsed
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("invalid file URL: {url}"))?;
        return ImageSource::File(&path).load();
    }

    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("unsupported image URL scheme: {}", parsed.scheme());
    }

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .context("failed to create image HTTP client")?;
    let response = client
        .get(parsed)
        .send()
        .context("failed to download theme image")?
        .error_for_status()
        .context("theme image server returned an error")?;

    if response
        .content_length()
        .is_some_and(|length| length > MAX_REMOTE_IMAGE_BYTES)
    {
        bail!("theme image is larger than 25 MiB");
    }

    let mut bytes = Vec::new();
    response
        .take(MAX_REMOTE_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("failed to read downloaded theme image")?;
    if bytes.len() as u64 > MAX_REMOTE_IMAGE_BYTES {
        bail!("theme image is larger than 25 MiB");
    }

    ImageSource::Bytes(&bytes).load()
}

#[cfg(test)]
mod tests {
    use image::{GenericImageView, RgbaImage};

    use super::*;

    const MIKU_PNG: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/images/hatsune_miku.png"
    ));

    #[test]
    fn loads_blob_source() {
        let mut cover =
            CoverArt::from_source(ImageSource::Bytes(MIKU_PNG), CoverArtRenderMode::Ascii)
                .expect("bundled PNG should load");
        cover
            .prepare(&Picker::halfblocks())
            .expect("ASCII cover preparation should succeed");

        assert!(cover.source_image.is_some());
        assert!(cover.protocol.is_none());
    }

    #[test]
    fn loads_local_path_and_file_url_the_same_way() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/images/hatsune_miku.png")
            .canonicalize()
            .expect("bundled image path should resolve");
        let file_url = reqwest::Url::from_file_path(&path)
            .expect("path should convert to file URL")
            .to_string();

        let from_path = CoverArt::from_location(
            path.to_str().expect("test path should be UTF-8"),
            CoverArtRenderMode::NormalImage,
        )
        .expect("local image should load");
        let from_url = CoverArt::from_location(&file_url, CoverArtRenderMode::NormalImage)
            .expect("file URL image should load");

        assert_eq!(
            from_path
                .source_image
                .as_ref()
                .map(DynamicImage::dimensions),
            from_url.source_image.as_ref().map(DynamicImage::dimensions),
        );
    }

    #[test]
    fn accepts_an_already_decoded_image() {
        let image = DynamicImage::ImageRgba8(RgbaImage::new(2, 3));
        let mut cover =
            CoverArt::from_source(ImageSource::Image(image), CoverArtRenderMode::NormalImage)
                .expect("decoded image should be accepted");
        cover
            .prepare(&Picker::halfblocks())
            .expect("normal image preparation should succeed");

        assert_eq!(
            cover.source_image.as_ref().map(DynamicImage::dimensions),
            Some((2, 3))
        );
        assert!(cover.protocol.is_some());
    }
}
