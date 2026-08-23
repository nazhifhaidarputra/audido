use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, ensure};

use crate::metadata::{AudioMetadata, ChannelLayout};

const META_MAGIC: &[u8; 8] = b"AUDPCM01";
const MAX_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct CachedPcmMetadata {
    pub url: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration: f32,
    pub title: Option<String>,
    pub author: Option<String>,
    pub sample_count: usize,
}

impl CachedPcmMetadata {
    pub fn from_audio(url: String, metadata: &AudioMetadata, sample_count: usize) -> Self {
        Self {
            url,
            sample_rate: metadata.sample_rate,
            channels: metadata.num_channels,
            duration: metadata.duration,
            title: metadata.title.clone(),
            author: metadata.author.clone(),
            sample_count,
        }
    }

    pub fn to_audio_metadata(&self) -> AudioMetadata {
        AudioMetadata {
            sample_rate: self.sample_rate,
            num_channels: self.channels,
            channel_layout: ChannelLayout::from_channels(self.channels),
            duration: self.duration,
            format: "youtube-stream".to_string(),
            title: self.title.clone(),
            author: self.author.clone(),
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct PcmCache {
    directory: PathBuf,
}

impl PcmCache {
    pub fn new(stream_cache_directory: &Path) -> Self {
        Self {
            directory: stream_cache_directory.join("pcm"),
        }
    }

    pub fn key(url: &str, target_sample_rate: Option<u32>) -> String {
        // Stable FNV-1a key: unlike DefaultHasher, this remains compatible
        // between processes and Rust releases.
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in url
            .bytes()
            .chain([0])
            .chain(target_sample_rate.unwrap_or(0).to_le_bytes())
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{hash:016x}")
    }

    pub fn open(
        &self,
        key: &str,
        expected_url: &str,
        target_sample_rate: Option<u32>,
    ) -> anyhow::Result<Option<(CachedPcmMetadata, File)>> {
        let paths = self.paths(key);
        if !paths.metadata.exists() || !paths.samples.exists() {
            return Ok(None);
        }

        let metadata = read_metadata(&paths.metadata).context("invalid PCM cache metadata")?;
        ensure!(metadata.url == expected_url, "PCM cache URL mismatch");
        if let Some(rate) = target_sample_rate {
            ensure!(
                metadata.sample_rate == rate,
                "PCM cache sample-rate mismatch"
            );
        }
        ensure!(metadata.channels > 0, "PCM cache has no audio channels");
        ensure!(metadata.sample_rate > 0, "PCM cache has no sample rate");
        ensure!(metadata.duration.is_finite() && metadata.duration > 0.0);

        let samples = File::open(&paths.samples).context("failed to open cached PCM samples")?;
        let expected_bytes = metadata
            .sample_count
            .checked_mul(size_of::<f32>())
            .context("cached PCM sample count overflow")? as u64;
        ensure!(
            samples.metadata()?.len() == expected_bytes,
            "cached PCM file length mismatch"
        );
        Ok(Some((metadata, samples)))
    }

    pub fn writer(&self, key: &str) -> anyhow::Result<PcmCacheWriter> {
        fs::create_dir_all(&self.directory)
            .with_context(|| format!("failed to create PCM cache at {:?}", self.directory))?;
        // `open` only calls this path for a miss. Clear any orphaned final or
        // partial file left by an interrupted process before starting anew.
        self.invalidate(key);
        let paths = self.paths(key);
        let file = File::create(&paths.samples_partial)
            .context("failed to create partial PCM cache file")?;
        Ok(PcmCacheWriter {
            paths,
            samples: Some(BufWriter::new(file)),
            sample_count: 0,
        })
    }

    pub fn invalidate(&self, key: &str) {
        let paths = self.paths(key);
        for path in [
            paths.metadata,
            paths.metadata_partial,
            paths.samples,
            paths.samples_partial,
        ] {
            if let Err(error) = fs::remove_file(&path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("Failed to remove invalid PCM cache file {path:?}: {error}");
            }
        }
    }

    fn paths(&self, key: &str) -> CachePaths {
        CachePaths {
            metadata: self.directory.join(format!("{key}.meta")),
            metadata_partial: self.directory.join(format!("{key}.meta.part")),
            samples: self.directory.join(format!("{key}.f32")),
            samples_partial: self.directory.join(format!("{key}.f32.part")),
        }
    }
}

#[derive(Debug)]
struct CachePaths {
    metadata: PathBuf,
    metadata_partial: PathBuf,
    samples: PathBuf,
    samples_partial: PathBuf,
}

pub(super) struct PcmCacheWriter {
    paths: CachePaths,
    samples: Option<BufWriter<File>>,
    sample_count: usize,
}

impl PcmCacheWriter {
    pub fn append(&mut self, samples: &[f32]) -> anyhow::Result<()> {
        let writer = self
            .samples
            .as_mut()
            .context("PCM cache writer already completed")?;
        for sample in samples {
            writer.write_all(&sample.to_le_bytes())?;
        }
        self.sample_count = self.sample_count.saturating_add(samples.len());
        Ok(())
    }

    pub fn sample_count(&self) -> usize {
        self.sample_count
    }

    pub fn finish(mut self, metadata: &CachedPcmMetadata) -> anyhow::Result<()> {
        ensure!(metadata.sample_count == self.sample_count);
        let mut samples = self.samples.take().context("missing PCM cache writer")?;
        samples.flush()?;
        samples.get_ref().sync_all()?;
        drop(samples);

        replace_file(&self.paths.samples_partial, &self.paths.samples)?;

        let metadata_file = File::create(&self.paths.metadata_partial)?;
        let mut metadata_writer = BufWriter::new(metadata_file);
        write_metadata(&mut metadata_writer, metadata)?;
        metadata_writer.flush()?;
        metadata_writer.get_ref().sync_all()?;
        drop(metadata_writer);
        replace_file(&self.paths.metadata_partial, &self.paths.metadata)?;
        Ok(())
    }
}

fn replace_file(source: &Path, destination: &Path) -> anyhow::Result<()> {
    // Renaming within the same cache directory commits each file atomically.
    fs::rename(source, destination)?;
    Ok(())
}

fn write_metadata(writer: &mut impl Write, metadata: &CachedPcmMetadata) -> anyhow::Result<()> {
    writer.write_all(META_MAGIC)?;
    writer.write_all(&metadata.sample_rate.to_le_bytes())?;
    writer.write_all(&metadata.channels.to_le_bytes())?;
    writer.write_all(&metadata.duration.to_le_bytes())?;
    writer.write_all(&(metadata.sample_count as u64).to_le_bytes())?;
    write_string(writer, Some(&metadata.url))?;
    write_string(writer, metadata.title.as_deref())?;
    write_string(writer, metadata.author.as_deref())?;
    Ok(())
}

fn read_metadata(path: &Path) -> anyhow::Result<CachedPcmMetadata> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut magic = [0_u8; META_MAGIC.len()];
    reader.read_exact(&mut magic)?;
    ensure!(&magic == META_MAGIC, "unsupported PCM cache format");

    let sample_rate = read_u32(&mut reader)?;
    let channels = read_u16(&mut reader)?;
    let duration = read_f32(&mut reader)?;
    let sample_count = usize::try_from(read_u64(&mut reader)?)?;
    let url = read_string(&mut reader)?.context("PCM cache is missing its source URL")?;
    let title = read_string(&mut reader)?;
    let author = read_string(&mut reader)?;

    Ok(CachedPcmMetadata {
        url,
        sample_rate,
        channels,
        duration,
        title,
        author,
        sample_count,
    })
}

fn write_string(writer: &mut impl Write, value: Option<&str>) -> anyhow::Result<()> {
    match value {
        Some(value) => {
            let bytes = value.as_bytes();
            ensure!(bytes.len() <= MAX_TEXT_BYTES);
            writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
            writer.write_all(bytes)?;
        }
        None => writer.write_all(&u32::MAX.to_le_bytes())?,
    }
    Ok(())
}

fn read_string(reader: &mut impl Read) -> anyhow::Result<Option<String>> {
    let length = read_u32(reader)?;
    if length == u32::MAX {
        return Ok(None);
    }
    let length = length as usize;
    ensure!(length <= MAX_TEXT_BYTES, "cached text field is too large");
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(Some(String::from_utf8(bytes)?))
}

fn read_u16(reader: &mut impl Read) -> anyhow::Result<u16> {
    let mut bytes = [0; size_of::<u16>()];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(reader: &mut impl Read) -> anyhow::Result<u32> {
    let mut bytes = [0; size_of::<u32>()];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> anyhow::Result<u64> {
    let mut bytes = [0; size_of::<u64>()];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_f32(reader: &mut impl Read) -> anyhow::Result<f32> {
    let mut bytes = [0; size_of::<f32>()];
    reader.read_exact(&mut bytes)?;
    Ok(f32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_pcm_cache_round_trips_and_partial_cache_is_ignored() {
        let temporary = tempfile::tempdir().unwrap();
        let cache = PcmCache::new(temporary.path());
        let url = "https://youtube.test/watch?v=cache-me";
        let key = PcmCache::key(url, Some(48_000));

        let mut partial = cache.writer(&key).unwrap();
        partial.append(&[0.1, 0.2]).unwrap();
        drop(partial);
        assert!(cache.open(&key, url, Some(48_000)).unwrap().is_none());

        let mut writer = cache.writer(&key).unwrap();
        let samples = [0.1, -0.25, 0.5, -0.75];
        writer.append(&samples).unwrap();
        let metadata = CachedPcmMetadata {
            url: url.to_string(),
            sample_rate: 48_000,
            channels: 2,
            duration: 1.0,
            title: Some("Cached title".to_string()),
            author: Some("Cached author".to_string()),
            sample_count: samples.len(),
        };
        writer.finish(&metadata).unwrap();

        let (loaded, mut file) = cache.open(&key, url, Some(48_000)).unwrap().unwrap();
        assert_eq!(loaded.title.as_deref(), Some("Cached title"));
        assert_eq!(loaded.sample_count, samples.len());
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        let decoded: Vec<f32> = bytes
            .chunks_exact(size_of::<f32>())
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect();
        assert_eq!(decoded, samples);
    }

    #[test]
    fn cache_key_changes_with_url_and_output_rate() {
        let url = "https://youtube.test/watch?v=one";
        assert_eq!(
            PcmCache::key(url, Some(48_000)),
            PcmCache::key(url, Some(48_000))
        );
        assert_ne!(
            PcmCache::key(url, Some(44_100)),
            PcmCache::key(url, Some(48_000))
        );
        assert_ne!(
            PcmCache::key(url, Some(48_000)),
            PcmCache::key("https://youtube.test/watch?v=two", Some(48_000))
        );
    }
}
