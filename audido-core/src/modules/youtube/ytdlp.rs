use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use rodio::{Decoder, Source};
use stream_download::{
    Settings, StreamDownload, http::HttpStream, storage::temp::TempStorageProvider,
};
use thiserror::Error;
use tokio::sync::OnceCell;
pub use yt_dlp::model::playlist::PlaylistEntry;
use yt_dlp::{VideoSelection, prelude::*};

use futures::stream::{self, StreamExt};

use super::cache::{CachedPcmMetadata, PcmCache};
use crate::{
    metadata::{AudioMetadata, ChannelLayout},
    modules::core::CHUNK_SIZE,
    source::{AudioBuffer, AudioPlaybackData, PositionTracker, StreamingAudioBuffer},
};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";
const HTTP_PREFETCH_BYTES: u64 = 16 * 1024;
const STARTUP_BUFFER_MILLIS: usize = 500;
const STARTUP_BUFFER_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct MemoryCachedAudio {
    metadata: AudioMetadata,
    buffer: StreamingAudioBuffer,
}

impl MemoryCachedAudio {
    fn playback_data(&self) -> AudioPlaybackData {
        playback_data(self.metadata.clone(), self.buffer.clone())
    }
}

/// Owns the yt-dlp/ffmpeg binary handle, the stream cache directory, and a
/// lazily-built `Downloader` shared across every search/stream call for the
/// life of the process. Lives on `CoreContext` (built once at `init()`),
/// but is self-contained enough to construct directly in tests without
/// spinning up the rest of the audio engine.
pub struct YtDlpRuntime {
    libraries: Libraries,
    cache_dir: PathBuf,
    pcm_cache: PcmCache,
    memory_cache: Mutex<HashMap<String, MemoryCachedAudio>>,
    load_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    downloader: OnceCell<Arc<Downloader>>,
}

impl YtDlpRuntime {
    pub fn new(libraries: Libraries, cache_dir: PathBuf) -> Self {
        let pcm_cache = PcmCache::new(&cache_dir);
        Self {
            libraries,
            cache_dir,
            pcm_cache,
            memory_cache: Mutex::new(HashMap::new()),
            load_locks: Mutex::new(HashMap::new()),
            downloader: OnceCell::new(),
        }
    }

    fn memory_cache_get(&self, key: &str) -> Option<MemoryCachedAudio> {
        self.memory_cache
            .lock()
            .expect("YouTube memory cache poisoned")
            .get(key)
            .cloned()
    }

    fn memory_cache_insert(&self, key: String, audio: MemoryCachedAudio) {
        self.memory_cache
            .lock()
            .expect("YouTube memory cache poisoned")
            .insert(key, audio);
    }

    fn memory_cache_remove(&self, key: &str) {
        self.memory_cache
            .lock()
            .expect("YouTube memory cache poisoned")
            .remove(key);
    }

    fn load_lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.load_locks
            .lock()
            .expect("YouTube load-lock map poisoned")
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    async fn ready_memory_cache(
        &self,
        key: &str,
    ) -> std::result::Result<Option<AudioPlaybackData>, YoutubeStreamError> {
        let Some(cached) = self.memory_cache_get(key) else {
            return Ok(None);
        };

        if let Err(error) = wait_for_startup_buffer(
            &cached.buffer,
            cached.metadata.sample_rate,
            cached.metadata.num_channels,
        )
        .await
        {
            self.memory_cache_remove(key);
            log::warn!("Discarding unusable YouTube memory cache {key}: {error}");
            return Ok(None);
        }

        log::info!("YouTube PCM memory cache hit: {key}");
        Ok(Some(cached.playback_data()))
    }

    /// Returns the shared `Downloader`, building it on first use only.
    /// Every subsequent call across the app reuses this same instance —
    /// no repeated binary verification, and yt-dlp's own on-disk caches
    /// (nsig cache, extractor cache) stay warm in `cache_dir` for the
    /// lifetime of the process.
    async fn downloader(&self) -> anyhow::Result<Arc<Downloader>> {
        self.downloader
            .get_or_try_init(|| async {
                DownloaderBuilder::new(self.libraries.clone(), self.cache_dir.clone())
                    .with_timeout(Duration::from_secs(30))
                    .with_args(vec![
                        "--no-playlist".to_string(),
                        "-f".to_string(),
                        "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/bestaudio[ext=mp3]/bestaudio"
                            .to_string(),
                        "--dump-json".to_string(),
                        "--user-agent".to_string(),
                        USER_AGENT.to_string(),
                        "--extractor-args".to_string(),
                        "youtube:player_client=android".to_string(),
                    ])
                    .build()
                    .await
                    .map(Arc::new)
                    .map_err(|e| anyhow::anyhow!(e))
            })
            .await
            .cloned()
    }

    /// Search for video based on query
    pub async fn search_youtube_by_query(
        &self,
        query: &str,
        max_items_per_page: usize,
        page_idx: usize,
    ) -> std::result::Result<Vec<PlaylistEntry>, YoutubeSearchError> {
        let fetch_count = max_items_per_page
            .saturating_mul(page_idx.saturating_add(1))
            .min(50);
        let downloader = self.downloader().await.map_err(|e| {
            log::error!("Failed to build yt_downloader for search: {e}");
            YoutubeSearchError::DownloaderBuildFailed(e.to_string())
        })?;
        let entries = downloader
            .youtube_extractor()
            .search(query, fetch_count)
            .await
            .map_err(|e| {
                log::error!("Failed to execute YouTube search: {e}");
                YoutubeSearchError::SearchRequestFailed(e.to_string())
            })?
            .entries;

        if entries.is_empty() {
            return Err(YoutubeSearchError::NoResults {
                query: query.to_string(),
            });
        }
        let start = page_idx.saturating_mul(max_items_per_page);
        let page: Vec<PlaylistEntry> = entries
            .into_iter()
            .skip(start)
            .take(max_items_per_page)
            .collect();
        if page.is_empty() {
            return Err(YoutubeSearchError::NoResults {
                query: query.to_string(),
            });
        }
        Ok(page)
    }

    /// Load youtube stream bytes chunk by chunk from a direct Youtube URL.
    ///
    /// The stream is handled to the Rodio Decoder and put inside
    /// The AudioPlaybackData which then read by the audio thread
    pub async fn load_youtube_stream(
        &self,
        url: &str,
        target_sample_rate: Option<u32>,
    ) -> std::result::Result<AudioPlaybackData, YoutubeStreamError> {
        let start_time = tokio::time::Instant::now();
        let cache_key = PcmCache::key(url, target_sample_rate);

        if let Some(cached) = self.ready_memory_cache(&cache_key).await? {
            return Ok(cached);
        }

        // Only one task may populate a given URL/rate cache entry. Different
        // songs still load concurrently during queue prefetch.
        let load_lock = self.load_lock(&cache_key);
        let _load_guard = load_lock.lock().await;
        if let Some(cached) = self.ready_memory_cache(&cache_key).await? {
            return Ok(cached);
        }

        match self.pcm_cache.open(&cache_key, url, target_sample_rate) {
            Ok(Some((cached_metadata, samples_file))) => {
                let metadata = cached_metadata.to_audio_metadata();
                let stream_buffer = StreamingAudioBuffer::new();
                load_cached_samples(
                    samples_file,
                    cached_metadata.sample_count,
                    stream_buffer.clone(),
                    self.pcm_cache.clone(),
                    cache_key.clone(),
                );
                if let Err(error) = wait_for_startup_buffer(
                    &stream_buffer,
                    metadata.sample_rate,
                    metadata.num_channels,
                )
                .await
                {
                    self.pcm_cache.invalidate(&cache_key);
                    return Err(error);
                }

                let cached = MemoryCachedAudio {
                    metadata,
                    buffer: stream_buffer,
                };
                let playback_data = cached.playback_data();
                self.memory_cache_insert(cache_key.clone(), cached);
                log::info!(
                    "YouTube PCM disk cache hit for {url}; ready in {:?}",
                    start_time.elapsed()
                );
                return Ok(playback_data);
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!("Ignoring invalid YouTube PCM cache {cache_key}: {error:#}");
                self.pcm_cache.invalidate(&cache_key);
            }
        }

        log::info!("Starting YouTube stream buffering for {url}");

        let downloader = self
            .downloader()
            .await
            .map_err(YoutubeStreamError::DownloaderBuildFailed)?;

        let video_info = downloader.fetch_video_infos(url).await.map_err(|e| {
            YoutubeStreamError::VideoInfoFetchFailed {
                url: url.to_string(),
                source: anyhow::anyhow!(e),
            }
        })?;

        let video_duration_secs = video_info.duration.unwrap_or(0);
        if video_duration_secs <= 0 || video_duration_secs >= u32::MAX as i64 {
            return Err(YoutubeStreamError::InvalidDuration {
                duration_secs: video_duration_secs,
            });
        }

        // Select the direct CDN-backed AAC format exposed by yt-dlp. Unlike
        // `download_audio_stream_with_quality`, this does not wait for the
        // entire file to be written before rodio can start decoding it.
        let audio_format = video_info
            .select_audio_format(AudioQuality::Best, AudioCodecPreference::AAC)
            .filter(|format| {
                format
                    .codec_info
                    .audio_codec
                    .as_deref()
                    .is_some_and(|codec| {
                        codec.to_ascii_lowercase().contains("aac")
                            || codec.to_ascii_lowercase().contains("mp4a")
                    })
            })
            .ok_or_else(|| YoutubeStreamError::NoPlayableFormat {
                url: url.to_string(),
            })?;
        let stream_url = audio_format
            .url()
            .map_err(|_| YoutubeStreamError::NoPlayableFormat {
                url: url.to_string(),
            })?
            .parse::<reqwest::Url>()
            .map_err(|e| YoutubeStreamError::StreamConnectFailed(anyhow::anyhow!(e)))?;

        // Preserve yt-dlp's request headers (especially User-Agent) for both
        // the initial request and any range requests made while rodio seeks.
        let client = reqwest::Client::builder()
            .default_headers(audio_format.download_info.http_headers.to_header_map())
            .build()
            .map_err(|e| YoutubeStreamError::StreamConnectFailed(anyhow::anyhow!(e)))?;
        let http_stream = HttpStream::new(client, stream_url)
            .await
            .map_err(|e| YoutubeStreamError::StreamConnectFailed(anyhow::anyhow!(e.to_string())))?;
        let stream = StreamDownload::from_stream(
            http_stream,
            TempStorageProvider::new_in(self.cache_dir.clone()),
            Settings::default()
                // stream-download blocks readers until this prefetch is met.
                // A smaller compressed-byte window lets rodio probe and begin
                // decoding quickly; the PCM startup window below protects
                // playback from underruns.
                .prefetch_bytes(HTTP_PREFETCH_BYTES)
                .retry_timeout(Duration::from_secs(10)),
        )
        .await
        .map_err(|e| YoutubeStreamError::StreamConnectFailed(anyhow::anyhow!(e.to_string())))?;

        log::info!(
            "HTTP audio stream buffered for decoding in {:?}",
            start_time.elapsed()
        );

        // Decoder probing performs blocking reads/seeks. Keep it off Tokio's
        // worker threads; unread bytes continue arriving in the background.
        let decoder = tokio::task::spawn_blocking(move || {
            Decoder::new(stream).map_err(|e| {
                log::error!("rodio Decoder::new failed on YouTube stream: '{e}'");
                std::io::Error::other(e)
            })
        })
        .await
        .map_err(|e| YoutubeStreamError::DecodeFailed(anyhow::anyhow!(e)))?
        .map_err(|e| YoutubeStreamError::DecodeFailed(anyhow::anyhow!(e)))?;

        let source_sample_rate = decoder.sample_rate();
        let num_channels = decoder.channels();
        let channel_layout = match num_channels {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            _ => ChannelLayout::Unsupported,
        };
        let target_rate = target_sample_rate.unwrap_or(source_sample_rate);
        let initial_metadata = AudioMetadata {
            sample_rate: target_rate,
            num_channels,
            channel_layout,
            duration: video_info.duration.unwrap_or(0) as f32,
            format: "youtube-stream".to_string(),
            title: Some(video_info.title.clone()),
            author: video_info.uploader.clone(),
            ..Default::default()
        };

        let video_duration_samples = video_duration_secs
            .saturating_mul(target_rate as i64)
            .saturating_mul(num_channels as i64) as usize;
        let (tx, rx) = crossbeam_channel::bounded::<f32>(target_rate as usize * 2 * 4);
        thread::spawn(move || {
            decode_youtube_stream(decoder, source_sample_rate, target_rate, num_channels, tx);
        });

        let stream_buffer = StreamingAudioBuffer::new();
        let expected_samples = video_duration_samples;
        let cache = self.pcm_cache.clone();
        let metadata_for_cache = initial_metadata.clone();
        let url_for_cache = url.to_string();
        let key_for_cache = cache_key.clone();
        let buffer_writer = stream_buffer.clone();
        thread::spawn(move || {
            retain_and_cache_stream(
                rx,
                buffer_writer,
                cache,
                key_for_cache,
                url_for_cache,
                metadata_for_cache,
                expected_samples,
            );
        });

        let cached = MemoryCachedAudio {
            metadata: initial_metadata,
            buffer: stream_buffer,
        };
        self.memory_cache_insert(cache_key.clone(), cached.clone());

        if let Err(error) = wait_for_startup_buffer(
            &cached.buffer,
            cached.metadata.sample_rate,
            cached.metadata.num_channels,
        )
        .await
        {
            self.memory_cache_remove(&cache_key);
            return Err(error);
        }

        let playback_data = cached.playback_data();
        log::debug!(
            "Stream connection and PCM startup buffer established in {:?}",
            start_time.elapsed()
        );
        Ok(playback_data)
    }

    /// Loads several YouTube URLs concurrently (capped at `concurrency`
    /// in-flight fetches) — mirrors the "prefetch next queue items" pattern
    /// Discord music bots use.
    pub async fn prefetch_youtube_streams(
        self: &Arc<Self>,
        urls: Vec<String>,
        target_sample_rate: Option<u32>,
        concurrency: usize,
    ) -> Vec<std::result::Result<AudioPlaybackData, YoutubeStreamError>> {
        stream::iter(urls.into_iter().map(|url| {
            let this = Arc::clone(self);
            async move { this.load_youtube_stream(&url, target_sample_rate).await }
        }))
        .buffer_unordered(concurrency.max(1))
        .collect()
        .await
    }
}

fn playback_data(metadata: AudioMetadata, buffer: StreamingAudioBuffer) -> AudioPlaybackData {
    let total_samples =
        (metadata.sample_rate as f32 * metadata.duration * metadata.num_channels as f32) as usize;
    let position_tracker =
        PositionTracker::new(total_samples, metadata.sample_rate, metadata.num_channels);
    AudioPlaybackData::new(metadata, AudioBuffer::Stream(buffer), position_tracker)
}

async fn wait_for_startup_buffer(
    buffer: &StreamingAudioBuffer,
    sample_rate: u32,
    channels: u16,
) -> std::result::Result<(), YoutubeStreamError> {
    let startup_samples = (sample_rate as usize)
        .saturating_mul(channels as usize)
        .saturating_mul(STARTUP_BUFFER_MILLIS)
        / 1000;
    let deadline = tokio::time::Instant::now() + STARTUP_BUFFER_TIMEOUT;

    loop {
        let buffered = buffer.buffered_samples();
        if buffered >= startup_samples.max(CHUNK_SIZE) {
            return Ok(());
        }
        if buffer.is_complete() {
            return if buffered > 0 {
                Ok(())
            } else {
                Err(YoutubeStreamError::DecodeFailed(anyhow::anyhow!(
                    "YouTube decoder produced no PCM samples"
                )))
            };
        }
        if tokio::time::Instant::now() >= deadline {
            return if buffered > 0 {
                Ok(())
            } else {
                Err(YoutubeStreamError::DecodeFailed(anyhow::anyhow!(
                    "YouTube decoder did not produce startup audio within {:?}",
                    STARTUP_BUFFER_TIMEOUT
                )))
            };
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn retain_and_cache_stream(
    rx: crossbeam_channel::Receiver<f32>,
    buffer: StreamingAudioBuffer,
    cache: PcmCache,
    cache_key: String,
    url: String,
    metadata: AudioMetadata,
    expected_samples: usize,
) {
    let mut cache_writer = match cache.writer(&cache_key) {
        Ok(writer) => Some(writer),
        Err(error) => {
            log::warn!("YouTube PCM disk cache is unavailable: {error:#}");
            None
        }
    };
    let mut batch = Vec::with_capacity(CHUNK_SIZE);

    while let Ok(first_sample) = rx.recv() {
        batch.push(first_sample);
        while batch.len() < CHUNK_SIZE {
            match rx.try_recv() {
                Ok(sample) => batch.push(sample),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            }
        }

        buffer.append(&batch);
        if let Some(writer) = cache_writer.as_mut()
            && let Err(error) = writer.append(&batch)
        {
            log::warn!("Failed writing YouTube PCM cache: {error:#}");
            cache_writer = None;
        }
        batch.clear();
    }
    buffer.mark_complete();

    let minimum_complete_samples = expected_samples.saturating_mul(9) / 10;
    let Some(writer) = cache_writer else {
        cache.invalidate(&cache_key);
        return;
    };
    if writer.sample_count() < minimum_complete_samples {
        log::warn!(
            "Not caching incomplete YouTube PCM stream: decoded {} of approximately {} samples",
            writer.sample_count(),
            expected_samples
        );
        cache.invalidate(&cache_key);
        return;
    }

    let cache_metadata = CachedPcmMetadata::from_audio(url, &metadata, writer.sample_count());
    match writer.finish(&cache_metadata) {
        Ok(()) => log::info!(
            "Cached {} YouTube PCM samples under key {cache_key}",
            cache_metadata.sample_count
        ),
        Err(error) => {
            log::warn!("Failed to finalize YouTube PCM cache: {error:#}");
            cache.invalidate(&cache_key);
        }
    }
}

fn load_cached_samples(
    mut samples_file: File,
    sample_count: usize,
    buffer: StreamingAudioBuffer,
    cache: PcmCache,
    cache_key: String,
) {
    thread::spawn(move || {
        let mut remaining = sample_count;
        let mut bytes = vec![0_u8; CHUNK_SIZE * size_of::<f32>()];
        let mut samples = Vec::with_capacity(CHUNK_SIZE);
        let result: std::io::Result<()> = (|| {
            while remaining > 0 {
                let samples_to_read = remaining.min(CHUNK_SIZE);
                let byte_count = samples_to_read * size_of::<f32>();
                samples_file.read_exact(&mut bytes[..byte_count])?;
                samples.clear();
                samples.extend(
                    bytes[..byte_count]
                        .chunks_exact(size_of::<f32>())
                        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four-byte PCM"))),
                );
                buffer.append(&samples);
                remaining -= samples_to_read;
            }
            Ok(())
        })();

        if let Err(error) = result {
            log::warn!("Failed reading cached YouTube PCM: {error}");
            cache.invalidate(&cache_key);
        }
        buffer.mark_complete();
    });
}

/// Distinguishes "the search infrastructure failed" from "the search
/// succeeded but returned nothing" so callers can react appropriately
/// (e.g. retry/alert on `DownloaderBuildFailed`/`SearchRequestFailed`,
/// but just show "no results" on `NoResults`).
#[derive(Debug, Error)]
pub enum YoutubeSearchError {
    #[error("Failed to build yt-dlp downloader: {0}")]
    DownloaderBuildFailed(String),

    #[error("YouTube search request failed: {0}")]
    SearchRequestFailed(String),

    #[error("Search succeeded but returned no entries for query {query:?}")]
    NoResults { query: String },
}

/// Errors that can occur while setting up a YouTube audio stream.
/// Kept as distinct variants (rather than folding everything into
/// `anyhow::Error` strings) so callers upstream can pattern-match on
/// failure category — e.g. retry on `StreamConnectFailed`, but surface
/// `NoPlayableFormat` or `InvalidDuration` as a user-facing "can't play this" message.
#[derive(Debug, Error)]
pub enum YoutubeStreamError {
    #[error("Failed to build yt-dlp downloader: {0}")]
    DownloaderBuildFailed(#[source] anyhow::Error),

    #[error("Failed to fetch video info for {url}: {source}")]
    VideoInfoFetchFailed {
        url: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("No playable audio format found for {url}")]
    NoPlayableFormat { url: String },

    #[error("Failed to start HTTP stream download: {0}")]
    StreamConnectFailed(#[source] anyhow::Error),

    #[error("Failed to decode audio stream: {0}")]
    DecodeFailed(#[source] anyhow::Error),

    #[error("Video reported invalid/out-of-range duration: {duration_secs}")]
    InvalidDuration { duration_secs: i64 },
}

/// Runs on a dedicated thread: pulls PCM samples from `decoder`, normalizes
/// them to `f32`, resampling on the way through if `source_sample_rate` and
/// `target_rate` differ, and pushes them onto `tx`.
fn decode_youtube_stream(
    decoder: Decoder<impl std::io::Read + std::io::Seek>,
    source_sample_rate: u32,
    target_rate: u32,
    num_channels: u16,
    tx: crossbeam_channel::Sender<f32>,
) {
    log::info!("YouTube decode thread started.");

    // rodio 0.21's Sample type is already normalized `f32` PCM. AAC streams
    // commonly begin with encoder-delay frames containing only silence; trim
    // those complete frames so playback and visualisation start with content.
    let samples = trim_leading_silence(decoder, num_channels);

    if target_rate != source_sample_rate {
        let (raw_tx, raw_rx) =
            crossbeam_channel::bounded::<f32>(CHUNK_SIZE * num_channels as usize * 4);

        thread::spawn(move || {
            if let Err(e) = crate::modules::resampler::resample_to_device_rate_stream(
                num_channels,
                source_sample_rate,
                target_rate,
                raw_rx,
                tx,
            ) {
                log::error!("Resampler thread failed: {e}");
            }
        });

        for sample in samples {
            if raw_tx.send(sample).is_err() {
                break;
            }
        }
    } else {
        for sample in samples {
            if tx.send(sample).is_err() {
                log::debug!("Stream receiver dropped, stopping decode thread.");
                break;
            }
        }
    }

    log::info!("YouTube decode thread finished.");
}

fn trim_leading_silence<I>(mut samples: I, num_channels: u16) -> impl Iterator<Item = f32>
where
    I: Iterator<Item = f32>,
{
    const SILENCE_THRESHOLD: f32 = 0.001;

    let channels = usize::from(num_channels.max(1));
    let mut first_audible_frame = Vec::with_capacity(channels);
    loop {
        first_audible_frame.clear();
        for _ in 0..channels {
            match samples.next() {
                Some(sample) => first_audible_frame.push(sample),
                None => return first_audible_frame.into_iter().chain(samples),
            }
        }

        if first_audible_frame
            .iter()
            .any(|sample| sample.abs() > SILENCE_THRESHOLD)
        {
            break;
        }
    }

    first_audible_frame.into_iter().chain(samples)
}

#[cfg(test)]
mod tests {
    use crate::modules::core::init_ytdlp_lib;

    use super::*;
    use log::info;
    use std::{collections::HashSet, sync::OnceLock};

    use tokio::time::{Instant, timeout};

    const TEST_TIMEOUT: Duration = Duration::from_secs(20);
    const TEST_VIDEO_URL: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
    const SHORT_TEST_VIDEO_URL: &str = "https://www.youtube.com/watch?v=ksL6sdTP70U";

    #[ctor::ctor(unsafe)]
    fn init_logging() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    fn test_runtime() -> &'static YtDlpRuntime {
        static RUNTIME: OnceLock<YtDlpRuntime> = OnceLock::new();

        RUNTIME.get_or_init(|| {
            info!(target: "test_runtime", "Initializing YtDlpRuntime...");

            let libraries = init_ytdlp_lib();
            info!(target: "test_runtime", "yt-dlp path: {:?}", libraries.youtube);
            info!(target: "test_runtime", "ffmpeg path: {:?}", libraries.ffmpeg);

            // Persistent, shared cache directory so yt-dlp's on-disk cache
            // stays warm across the whole test run.
            let cache_dir = std::env::temp_dir().join("audido_ytdlp_test_cache");
            std::fs::create_dir_all(&cache_dir).unwrap();
            info!(target: "test_runtime", "Cache directory: {:?}", cache_dir);

            let runtime = YtDlpRuntime::new(libraries, cache_dir);
            info!(target: "test_runtime", "YtDlpRuntime initialized successfully");
            runtime
        })
    }

    // ==================== Search Integration Tests ====================

    #[tokio::test]
    async fn test_search_youtube_returns_results() {
        info!(target: "test_search_youtube_returns_results", "Starting test");
        let start = Instant::now();

        let result = timeout(
            TEST_TIMEOUT,
            test_runtime().search_youtube_by_query("rick astley never gonna give you up", 5, 0),
        )
        .await
        .expect("search timed out");

        info!(target: "test_search_youtube_returns_results", "Search completed in {:?}", start.elapsed());

        let entries = result.expect("search failed");
        info!(target: "test_search_youtube_returns_results", "Received {} entries", entries.len());

        assert!(!entries.is_empty());
        assert!(entries.len() <= 5);

        for (i, entry) in entries.iter().enumerate() {
            info!(target: "test_search_youtube_returns_results", "Entry {}: {} - {}", i + 1, entry.title, entry.url);
            assert!(!entry.title.is_empty());
            assert!(!entry.url.is_empty());
        }

        info!(target: "test_search_youtube_returns_results", "PASSED in {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn test_search_youtube_first_page_has_expected_content() {
        info!(target: "test_search_youtube_first_page_has_expected_content", "Starting test");
        let start = Instant::now();

        let result = timeout(
            TEST_TIMEOUT,
            test_runtime().search_youtube_by_query("lofi hip hop radio", 3, 0),
        )
        .await
        .expect("search timed out");

        info!(target: "test_search_youtube_first_page_has_expected_content", "Search completed in {:?}", start.elapsed());

        let entries = result.expect("search failed");
        info!(target: "test_search_youtube_first_page_has_expected_content", "Received {} entries", entries.len());

        assert_eq!(entries.len(), 3, "Should return exactly 3 entries");

        let unique_urls: HashSet<&str> = entries.iter().map(|entry| entry.url.as_str()).collect();

        assert_eq!(unique_urls.len(), 3, "All entries should have unique URLs");

        info!(target: "test_search_youtube_first_page_has_expected_content", "PASSED in {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn test_search_youtube_second_page_differs_from_first() {
        info!(target: "test_search_youtube_second_page_differs_from_first", "Starting test");
        let start = Instant::now();

        info!(target: "test_search_youtube_second_page_differs_from_first", "Fetching page 1...");
        let page1_start = Instant::now();
        let page1 = timeout(
            TEST_TIMEOUT,
            test_runtime().search_youtube_by_query("pop music", 3, 0),
        )
        .await
        .expect("page 1 timed out")
        .expect("page 1 search failed");
        info!(target: "test_search_youtube_second_page_differs_from_first", "Page 1 fetched in {:?}, got {} entries", page1_start.elapsed(), page1.len());

        info!(target: "test_search_youtube_second_page_differs_from_first", "Fetching page 2...");
        let page2_start = Instant::now();
        let page2 = timeout(
            TEST_TIMEOUT,
            test_runtime().search_youtube_by_query("pop music", 3, 1),
        )
        .await
        .expect("page 2 timed out")
        .expect("page 2 search failed");
        info!(target: "test_search_youtube_second_page_differs_from_first", "Page 2 fetched in {:?}, got {} entries", page2_start.elapsed(), page2.len());

        let page1_urls: HashSet<&str> = page1.iter().map(|entry| entry.url.as_str()).collect();
        let page2_urls: HashSet<&str> = page2.iter().map(|entry| entry.url.as_str()).collect();

        assert!(
            page2_urls.difference(&page1_urls).next().is_some(),
            "page 2 should introduce at least one new URL"
        );

        info!(target: "test_search_youtube_second_page_differs_from_first", "PASSED in {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn test_search_youtube_no_results_for_nonsense_query() {
        info!(target: "test_search_youtube_no_results_for_nonsense_query", "Starting test");
        let start = Instant::now();

        let nonsense_query = "xzqwkjlmnopqrstuvwxyz1234567890nonexistent";
        info!(target: "test_search_youtube_no_results_for_nonsense_query", "Searching for nonsense query: {}", nonsense_query);

        let result = timeout(
            TEST_TIMEOUT,
            test_runtime().search_youtube_by_query(nonsense_query, 5, 0),
        )
        .await
        .expect("search timed out");

        info!(target: "test_search_youtube_no_results_for_nonsense_query", "Search completed in {:?}", start.elapsed());

        match result {
            Err(YoutubeSearchError::NoResults { query }) => {
                info!(target: "test_search_youtube_no_results_for_nonsense_query", "Got expected NoResults error");
                assert_eq!(query, nonsense_query);
            }
            Ok(entries) => {
                info!(target: "test_search_youtube_no_results_for_nonsense_query", "Got {} entries (unexpected)", entries.len());
                for entry in entries {
                    let title_lower = entry.title.to_lowercase();

                    assert!(
                        !title_lower.contains("xzqwkjlmnopqrstuvwxyz"),
                        "Fallback result unexpectedly matched the nonsense query: {}",
                        title_lower
                    );
                }
            }
            Err(other) => {
                panic!("Expected NoResults error, but got: {:?}", other);
            }
        }

        info!(target: "test_search_youtube_no_results_for_nonsense_query", "PASSED in {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn test_search_youtube_page_beyond_results() {
        info!(target: "test_search_youtube_page_beyond_results", "Starting test");
        let start = Instant::now();

        info!(target: "test_search_youtube_page_beyond_results", "Fetching page 100 of 'cat videos'...");
        let result = timeout(
            TEST_TIMEOUT,
            test_runtime().search_youtube_by_query("cat videos", 10, 100),
        )
        .await
        .expect("search timed out");

        info!(target: "test_search_youtube_page_beyond_results", "Search completed in {:?}", start.elapsed());

        match result {
            Err(YoutubeSearchError::NoResults { .. }) => {
                info!(target: "test_search_youtube_page_beyond_results", "Got expected NoResults error");
                // Expected.
            }
            Ok(entries) => {
                panic!(
                    "Expected NoResults for page 100, but got {} entries",
                    entries.len()
                );
            }
            Err(other) => {
                panic!("Expected NoResults, but got: {:?}", other);
            }
        }

        info!(target: "test_search_youtube_page_beyond_results", "PASSED in {:?}", start.elapsed());
    }

    // ==================== Stream Loading Integration Tests ====================

    #[tokio::test]
    async fn test_load_youtube_stream_retrieves_metadata() {
        info!(target: "test_load_youtube_stream_retrieves_metadata", "Starting test");
        let start = Instant::now();

        info!(target: "test_load_youtube_stream_retrieves_metadata", "Loading stream for: {}", TEST_VIDEO_URL);
        let playback_data = timeout(
            TEST_TIMEOUT,
            test_runtime().load_youtube_stream(TEST_VIDEO_URL, Some(44_100)),
        )
        .await
        .expect("stream load timed out")
        .expect("stream load failed");

        info!(target: "test_load_youtube_stream_retrieves_metadata", "Stream loaded in {:?}", start.elapsed());

        let metadata = playback_data.metadata();
        info!(target: "test_load_youtube_stream_retrieves_metadata", "Metadata: sample_rate={}, channels={}, duration={:.2}s, title={:?}",
                 metadata.sample_rate, metadata.num_channels, metadata.duration, metadata.title);

        assert_eq!(metadata.sample_rate, 44_100);
        assert!(metadata.num_channels > 0);
        assert!(metadata.duration > 0.0);
        assert!(metadata.title.is_some());
        assert_eq!(metadata.format, "youtube-stream");

        assert!(matches!(
            metadata.channel_layout,
            ChannelLayout::Stereo | ChannelLayout::Mono
        ));

        assert_eq!(playback_data.position_tracker().position_seconds(), 0.0);

        info!(target: "test_load_youtube_stream_retrieves_metadata", "PASSED in {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn test_load_youtube_stream_produces_audio_samples() {
        info!(target: "test_load_youtube_stream_produces_audio_samples", "Starting test");
        let start = Instant::now();

        info!(target: "test_load_youtube_stream_produces_audio_samples", "Loading stream for: {}", TEST_VIDEO_URL);
        let playback_data = timeout(
            TEST_TIMEOUT,
            test_runtime().load_youtube_stream(TEST_VIDEO_URL, Some(44_100)),
        )
        .await
        .expect("stream load timed out")
        .expect("stream load failed");

        info!(target: "test_load_youtube_stream_produces_audio_samples", "Stream loaded in {:?}", start.elapsed());

        let buffer = playback_data.buffer();

        match buffer {
            AudioBuffer::Stream(stream_buffer) => {
                info!(target: "test_load_youtube_stream_produces_audio_samples", "Waiting for retained audio samples...");
                let deadline = std::time::Instant::now() + Duration::from_secs(10);

                while stream_buffer.buffered_samples() < 1000
                    && std::time::Instant::now() < deadline
                    && !stream_buffer.is_complete()
                {
                    std::thread::sleep(Duration::from_millis(20));
                }

                let mut received_samples = Vec::with_capacity(1000);
                stream_buffer.copy_from(0, 1000, &mut received_samples);
                for sample in &received_samples {
                    assert!(
                        (-1.0..=1.0).contains(sample),
                        "Sample {} out of valid range [-1, 1]",
                        sample
                    );
                }

                info!(target: "test_load_youtube_stream_produces_audio_samples", "Total samples received: {}", received_samples.len());

                assert!(
                    !received_samples.is_empty(),
                    "Should receive at least some audio samples"
                );

                let first = received_samples[0];
                let has_variation = received_samples
                    .iter()
                    .any(|&sample| (sample - first).abs() > 0.001);

                assert!(
                    has_variation,
                    "Audio samples should have variation, not all be identical"
                );
            }
            _ => panic!("Expected Stream buffer type"),
        }

        info!(target: "test_load_youtube_stream_produces_audio_samples", "PASSED in {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn test_repeated_load_reuses_the_live_pcm_buffer() {
        let first = timeout(
            TEST_TIMEOUT,
            test_runtime().load_youtube_stream(TEST_VIDEO_URL, Some(44_100)),
        )
        .await
        .expect("first stream load timed out")
        .expect("first stream load failed");

        let second = timeout(
            Duration::from_secs(2),
            test_runtime().load_youtube_stream(TEST_VIDEO_URL, Some(44_100)),
        )
        .await
        .expect("cached stream replay should not invoke yt-dlp")
        .expect("cached stream replay failed");

        match (first.buffer(), second.buffer()) {
            (AudioBuffer::Stream(first), AudioBuffer::Stream(second)) => {
                assert!(
                    first.shares_storage_with(second),
                    "queue replay should share the retained PCM allocation"
                );
            }
            _ => panic!("Expected streaming buffers"),
        }
    }

    #[tokio::test]
    async fn test_completed_stream_reloads_from_persistent_pcm_cache() {
        let runtime = test_runtime();
        let cache_key = PcmCache::key(SHORT_TEST_VIDEO_URL, Some(44_100));
        runtime.memory_cache_remove(&cache_key);
        runtime.pcm_cache.invalidate(&cache_key);

        let first = timeout(
            TEST_TIMEOUT,
            runtime.load_youtube_stream(SHORT_TEST_VIDEO_URL, Some(44_100)),
        )
        .await
        .expect("short stream load timed out")
        .expect("short stream load failed");
        let first_buffer = match first.buffer() {
            AudioBuffer::Stream(buffer) => buffer,
            _ => panic!("Expected streaming buffer"),
        };

        timeout(TEST_TIMEOUT, async {
            while !first_buffer.is_complete() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("short fixture did not finish decoding");

        // The collector marks the buffer complete just before atomically
        // finalizing the cache files, so allow that final filesystem step.
        timeout(TEST_TIMEOUT, async {
            loop {
                if runtime
                    .pcm_cache
                    .open(&cache_key, SHORT_TEST_VIDEO_URL, Some(44_100))
                    .ok()
                    .flatten()
                    .is_some()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("completed stream was not persisted");

        runtime.memory_cache_remove(&cache_key);
        let second = timeout(
            Duration::from_secs(2),
            runtime.load_youtube_stream(SHORT_TEST_VIDEO_URL, Some(44_100)),
        )
        .await
        .expect("persistent cache replay should bypass yt-dlp")
        .expect("persistent cache replay failed");
        let second_buffer = match second.buffer() {
            AudioBuffer::Stream(buffer) => buffer,
            _ => panic!("Expected streaming buffer"),
        };
        assert!(second_buffer.buffered_samples() >= 44_100 / 2);
        assert!(!first_buffer.shares_storage_with(second_buffer));
    }

    #[tokio::test]
    async fn test_load_youtube_stream_respects_target_sample_rate() {
        info!(target: "test_load_youtube_stream_respects_target_sample_rate", "Starting test");
        let start = Instant::now();

        info!(target: "test_load_youtube_stream_respects_target_sample_rate", "Loading stream with target rate 48000 Hz");
        let playback_data = timeout(
            TEST_TIMEOUT,
            test_runtime().load_youtube_stream(TEST_VIDEO_URL, Some(48_000)),
        )
        .await
        .expect("stream load timed out")
        .expect("stream load failed");

        info!(target: "test_load_youtube_stream_respects_target_sample_rate", "Stream loaded in {:?}", start.elapsed());

        assert_eq!(
            playback_data.metadata().sample_rate,
            48_000,
            "Should use requested 48000 Hz sample rate"
        );

        info!(target: "test_load_youtube_stream_respects_target_sample_rate", "PASSED in {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn test_load_youtube_stream_uses_source_rate_when_no_target() {
        info!(target: "test_load_youtube_stream_uses_source_rate_when_no_target", "Starting test");
        let start = Instant::now();

        info!(target: "test_load_youtube_stream_uses_source_rate_when_no_target", "Loading stream without target rate");
        let playback_data = timeout(
            TEST_TIMEOUT,
            test_runtime().load_youtube_stream(TEST_VIDEO_URL, None),
        )
        .await
        .expect("stream load timed out")
        .expect("stream load failed");

        info!(target: "test_load_youtube_stream_uses_source_rate_when_no_target", "Stream loaded in {:?}", start.elapsed());

        let rate = playback_data.metadata().sample_rate;
        info!(target: "test_load_youtube_stream_uses_source_rate_when_no_target", "Detected source sample rate: {}", rate);

        assert!(
            matches!(rate, 44_100 | 48_000 | 22_050 | 16_000),
            "Sample rate {} should be a common audio rate",
            rate
        );

        info!(target: "test_load_youtube_stream_uses_source_rate_when_no_target", "PASSED in {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn test_load_youtube_stream_channel_layout_detected() {
        info!(target: "test_load_youtube_stream_channel_layout_detected", "Starting test");
        let start = Instant::now();

        info!(target: "test_load_youtube_stream_channel_layout_detected", "Loading stream to detect channel layout");
        let playback_data = timeout(
            TEST_TIMEOUT,
            test_runtime().load_youtube_stream(TEST_VIDEO_URL, Some(44_100)),
        )
        .await
        .expect("stream load timed out")
        .expect("stream load failed");

        info!(target: "test_load_youtube_stream_channel_layout_detected", "Stream loaded in {:?}", start.elapsed());

        let layout = &playback_data.metadata().channel_layout;
        info!(target: "test_load_youtube_stream_channel_layout_detected", "Detected channel layout: {:?}", layout);

        assert!(
            matches!(layout, ChannelLayout::Stereo | ChannelLayout::Mono),
            "Expected Stereo or Mono, got {:?}",
            layout
        );

        info!(target: "test_load_youtube_stream_channel_layout_detected", "PASSED in {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn test_load_youtube_stream_invalid_url_fails() {
        info!(target: "test_load_youtube_stream_invalid_url_fails", "Starting test");
        let start = Instant::now();

        let invalid_url = "https://www.youtube.com/watch?v=INVALID_VIDEO_ID_12345";
        info!(target: "test_load_youtube_stream_invalid_url_fails", "Testing with invalid URL: {}", invalid_url);

        let result = timeout(
            TEST_TIMEOUT,
            test_runtime().load_youtube_stream(invalid_url, Some(44_100)),
        )
        .await
        .expect("stream load timed out");

        info!(target: "test_load_youtube_stream_invalid_url_fails", "Request completed in {:?}", start.elapsed());

        assert!(result.is_err(), "Should fail for invalid video URL");

        match result.unwrap_err() {
            YoutubeStreamError::VideoInfoFetchFailed { url, .. } => {
                info!(target: "test_load_youtube_stream_invalid_url_fails", "Got expected VideoInfoFetchFailed error");
                assert_eq!(url, invalid_url);
            }
            other => {
                panic!("Expected VideoInfoFetchFailed, got: {:?}", other);
            }
        }

        info!(target: "test_load_youtube_stream_invalid_url_fails", "PASSED in {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn test_load_youtube_stream_malformed_url_fails() {
        info!(target: "test_load_youtube_stream_malformed_url_fails", "Starting test");
        let start = Instant::now();

        let malformed_url = "not-a-valid-url";
        info!(target: "test_load_youtube_stream_malformed_url_fails", "Testing with malformed URL: {}", malformed_url);

        let result = timeout(
            TEST_TIMEOUT,
            test_runtime().load_youtube_stream(malformed_url, Some(44_100)),
        )
        .await
        .expect("stream load timed out");

        info!(target: "test_load_youtube_stream_malformed_url_fails", "Request completed in {:?}", start.elapsed());

        assert!(result.is_err(), "Should fail for malformed URL");

        info!(target: "test_load_youtube_stream_malformed_url_fails", "PASSED in {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn test_load_youtube_stream_position_tracker_initialized() {
        info!(target: "test_load_youtube_stream_position_tracker_initialized", "Starting test");
        let start = Instant::now();

        info!(target: "test_load_youtube_stream_position_tracker_initialized", "Loading stream to check position tracker");
        let playback_data = timeout(
            TEST_TIMEOUT,
            test_runtime().load_youtube_stream(TEST_VIDEO_URL, Some(44_100)),
        )
        .await
        .expect("stream load timed out")
        .expect("stream load failed");

        info!(target: "test_load_youtube_stream_position_tracker_initialized", "Stream loaded in {:?}", start.elapsed());

        assert_eq!(
            playback_data.position_tracker().position_seconds(),
            0.0,
            "Initial position should be 0"
        );

        info!(target: "test_load_youtube_stream_position_tracker_initialized", "PASSED in {:?}", start.elapsed());
    }

    // ==================== End-to-End Test ====================

    #[tokio::test]
    async fn test_search_then_play_workflow() {
        info!(target: "test_search_then_play_workflow", "Starting test");
        let start = Instant::now();

        info!(target: "test_search_then_play_workflow", "Step 1: Searching for video...");
        let search_start = Instant::now();
        let search_results = timeout(
            TEST_TIMEOUT,
            test_runtime().search_youtube_by_query("never gonna give you up rick astley", 1, 0),
        )
        .await
        .expect("search timed out")
        .expect("search should succeed");

        info!(target: "test_search_then_play_workflow", "Search completed in {:?}, found {} results", search_start.elapsed(), search_results.len());

        assert!(!search_results.is_empty(), "Search should return results");

        let video = &search_results[0];
        let video_url = &video.url;
        let video_title = &video.title;

        info!(target: "test_search_then_play_workflow", "Found video: {} at {}", video_title, video_url);

        info!(target: "test_search_then_play_workflow", "Step 2: Loading stream...");
        let stream_start = Instant::now();
        let playback_data = timeout(
            TEST_TIMEOUT,
            test_runtime().load_youtube_stream(video_url, Some(44_100)),
        )
        .await
        .expect("stream load timed out")
        .expect("stream load failed");

        info!(target: "test_search_then_play_workflow", "Stream loaded in {:?}", stream_start.elapsed());

        let metadata = playback_data.metadata();
        info!(target: "test_search_then_play_workflow", "Loaded metadata: title={:?}, duration={:.2}s", metadata.title, metadata.duration);

        let loaded_title = metadata
            .title
            .as_deref()
            .expect("Loaded video should have a title")
            .to_lowercase();

        let search_title = video_title.to_lowercase();

        assert!(
            loaded_title.contains("rick astley") || loaded_title.contains("never gonna"),
            "Loaded title '{}' should relate to search query, searched for '{}'",
            loaded_title,
            search_title
        );

        info!(target: "test_search_then_play_workflow", "Step 3: Receiving audio samples...");
        match playback_data.buffer() {
            AudioBuffer::Stream(stream_buffer) => {
                let sample_start = Instant::now();
                let deadline = std::time::Instant::now() + Duration::from_secs(15);
                while stream_buffer.buffered_samples() == 0
                    && std::time::Instant::now() < deadline
                    && !stream_buffer.is_complete()
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
                let mut samples = Vec::with_capacity(1);
                stream_buffer.copy_from(0, 1, &mut samples);
                let sample = *samples
                    .first()
                    .expect("Should be able to retain at least one sample");

                info!(target: "test_search_then_play_workflow", "First sample received in {:?}: {}", sample_start.elapsed(), sample);

                assert!(
                    (-1.0..=1.0).contains(&sample),
                    "Sample should be in valid range"
                );
            }
            _ => panic!("Expected Stream buffer type"),
        }

        info!(target: "test_search_then_play_workflow", "PASSED in {:?}", start.elapsed());
    }
}
