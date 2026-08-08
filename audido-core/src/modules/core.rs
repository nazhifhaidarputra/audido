//! # Core Audio Runtime
//!
//! Defines [`CoreContext`] and [`CoreHandle`], the central structs that hold all shared audio
//! engine state and provide the public API consumed by `audido-tui` routes.
//!
//! ## Architecture
//!
//! ```text
//!  ┌─────────────┐          Arc<CoreContext>          ┌──────────────────────────────┐
//!  │  audido-tui │ ─────────────────────────────────► │     CPAL Audio Thread        │
//!  │  (UI Event) │                                    │  cpal::Stream data callback  │
//!  │             │ ─ module fn call ──► tokio task ─► │  reads f32 from HeapConsumer │
//!  └─────────────┘                                    └──────────────────────────────┘
//!         │          CoreEvent broadcast                       ▲
//!         ◄────────────────────────────────────────────────────┘
//!                   (tokio::sync::broadcast)
//! ```
//!
//! All public module functions (`playback::play`, `queue::add_to_queue`, etc.) take
//! `Arc<CoreContext>` and spawn a Tokio task internally. This means callers on the TUI
//! thread are **never blocked**.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
};

use anyhow::Context;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::Sender;
use ringbuf::{HeapCons, HeapProd, HeapRb, traits::Split};
use tokio::sync::broadcast;

use crate::{
    commands::{CoreEvent, RealtimeCommand},
    dsp::{eq::Equalizer, normalization::Normalizer},
    queue::PlaybackQueue,
    source::AudioPlaybackData,
};
// ================================
// ========== Constants ===========
// ================================

/// Capacity of the SPSC ring buffer in f32 samples.
/// 16 384 samples @ 44100 Hz stereo ≈ ~185 ms of headroom before underrun.
pub const RING_BUFFER_CAPACITY: usize = 16_384;

/// Size of each DSP processing chunk in samples.
pub const CHUNK_SIZE: usize = 512;

/// Broadcast channel capacity for [`CoreEvent`].
const EVENT_BROADCAST_CAPACITY: usize = 256;

/// Atomics for lock-free reads/writes from both the UI task and CPAL audio thread.
pub struct PlaybackAtomics {
    /// `true` when playback is active (not paused, not stopped).
    pub is_playing: AtomicBool,
    /// Current playback position in samples (updated by DSP feed loop).
    pub position_samples: AtomicUsize,
    /// Total audio duration in samples (set when track is loaded).
    pub total_samples: AtomicUsize,
    /// Sample rate of the currently loaded track.
    pub sample_rate: AtomicU32,
    /// Number of channels of the currently loaded track.
    pub num_channels: AtomicU32,
    /// Fixed sample rate of the CPAL output device/stream. Tracks are
    /// pre-resampled to this rate at load time so no runtime resampling is
    /// needed in the DSP feed loop.
    pub device_sample_rate: AtomicU32,
    /// Volume target [0.0, 1.0] stored as f32 bits.
    pub volume: AtomicU32,
    /// Playback speed multiplier stored as f32 bits.
    pub speed: AtomicU32,
}

impl Default for PlaybackAtomics {
    fn default() -> Self {
        Self {
            is_playing: AtomicBool::new(false),
            position_samples: AtomicUsize::new(0),
            total_samples: AtomicUsize::new(0),
            sample_rate: AtomicU32::new(44100),
            num_channels: AtomicU32::new(2),
            device_sample_rate: AtomicU32::new(44100),
            volume: AtomicU32::new(f32::to_bits(1.0)),
            speed: AtomicU32::new(f32::to_bits(1.0)),
        }
    }
}

impl PlaybackAtomics {
    pub fn get_volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }
    pub fn set_volume(&self, v: f32) {
        self.volume.store(f32::to_bits(v), Ordering::Relaxed);
    }
    pub fn get_speed(&self) -> f32 {
        f32::from_bits(self.speed.load(Ordering::Relaxed))
    }
    pub fn set_speed(&self, s: f32) {
        self.speed.store(f32::to_bits(s), Ordering::Relaxed);
    }
}

/// Central shared state for the audio core.  
/// Wrapped in `Arc<CoreContext>` and passed to every module function and the CPAL thread.
pub struct CoreContext {
    // ── Playback atomics (lock-free, shared between UI tasks and CPAL thread) ──
    pub atomics: PlaybackAtomics,

    // ── Mutable engine state (behind Mutex — only modified by Tokio tasks) ──
    pub queue: Mutex<PlaybackQueue>,
    pub current_audio: Mutex<Option<AudioPlaybackData>>,
    pub eq_shadow: Mutex<Equalizer>,
    pub normalizer_shadow: Mutex<Normalizer>,
    pub eq_enabled: AtomicBool,
    pub normalizer_enabled: AtomicBool,

    // ── SPSC Ring Buffer Producer (used by DSP feed loop) ──
    /// The producer side of the HeapRb; only one DSP task at a time writes to this.
    /// Wrapped in Mutex so it can be taken/replaced when a new track starts.
    pub ring_producer: Mutex<HeapProd<f32>>,

    // ── Real-time command channel (to DSP processing task) ──
    /// Sender for [`RealtimeCommand`]s forwarded to the audio processing thread.
    pub rt_cmd_tx: Mutex<Option<Sender<RealtimeCommand>>>,

    // ── Event broadcast ──
    /// All subscribers receive a clone of each [`CoreEvent`] emitted.
    pub event_tx: broadcast::Sender<CoreEvent>,

    // ── Tokio runtime handle ──
    /// Background async runtime. All module functions use this to spawn tasks.
    pub tokio_handle: tokio::runtime::Handle,
}

impl CoreContext {
    fn new(
        event_tx: broadcast::Sender<CoreEvent>,
        tokio_handle: tokio::runtime::Handle,
        ring_producer: HeapProd<f32>,
    ) -> Self {
        Self {
            atomics: PlaybackAtomics::default(),
            queue: Mutex::new(PlaybackQueue::new()),
            current_audio: Mutex::new(None),
            eq_shadow: Mutex::new(Equalizer::new(44100, 2)),
            normalizer_shadow: Mutex::new(Normalizer::new()),
            eq_enabled: AtomicBool::new(false),
            normalizer_enabled: AtomicBool::new(false),
            ring_producer: Mutex::new(ring_producer),
            rt_cmd_tx: Mutex::new(None),
            event_tx,
            tokio_handle,
        }
    }

    /// Convenience: emit a [`CoreEvent`] to all subscribers (ignores send errors when
    /// there are no active receivers).
    pub fn emit(&self, event: CoreEvent) {
        let _ = self.event_tx.send(event);
    }
}

/// Handle returned to `audido-tui` after initialisation.  
/// Keeps the Tokio runtime alive (via `Arc`) and exposes the subscribe method for
/// receiving [`CoreEvent`]s.
pub struct CoreHandle {
    /// Shared engine context accessible by every module function.
    pub ctx: Arc<CoreContext>,
    /// Keep runtime alive for the duration of the program.
    _runtime: Arc<tokio::runtime::Runtime>,
    /// Keep CPAL stream alive — dropping it silences output.
    pub stream: Option<cpal::Stream>,
}

impl CoreHandle {
    /// Subscribe to the broadcast event channel.  
    /// Each call returns an independent receiver; all receive the same events.
    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.ctx.event_tx.subscribe()
    }

    /// Convenience: arc-clone the context for use in module calls.
    pub fn ctx(&self) -> Arc<CoreContext> {
        Arc::clone(&self.ctx)
    }
}

/// Initialise the audio core.  
/// This function:
/// 1. Opens the default CPAL output device and stream.
/// 2. Allocates the SPSC heap ring buffer — consumer goes to CPAL, producer stays in `CoreContext`.
/// 3. Starts the Tokio multi-thread runtime.
/// 4. Starts the position-update watcher Tokio task.
/// 5. Returns a [`CoreHandle`] ready for consumption by `audido-tui`.
pub fn init() -> anyhow::Result<CoreHandle> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("No default audio output device found")?;

    let config = device
        .default_output_config()
        .context("Failed to get default output config")?;

    log::info!(
        "CPAL device: '{}', format: {:?}, sample rate: {}, channels: {}",
        device.name().unwrap_or_default(),
        config.sample_format(),
        config.sample_rate().0,
        config.channels()
    );

    let ring = HeapRb::<f32>::new(RING_BUFFER_CAPACITY);
    let (producer, consumer) = ring.split();

    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .context("Failed to create Tokio runtime")?,
    );
    let tokio_handle = runtime.handle().clone();

    let (event_tx, _) = broadcast::channel(EVENT_BROADCAST_CAPACITY);
    let ctx = Arc::new(CoreContext::new(event_tx, tokio_handle.clone(), producer));
    ctx.atomics
        .device_sample_rate
        .store(config.sample_rate().0, Ordering::Relaxed);

    let cpal_stream = build_cpal_stream(&device, &config, consumer, Arc::clone(&ctx))
        .context("Failed to build CPAL output stream")?;

    cpal_stream.play().context("Failed to start CPAL stream")?;

    let ctx_for_watcher = Arc::clone(&ctx);
    tokio_handle.spawn(async move {
        run_position_watcher(ctx_for_watcher).await;
    });

    log::info!("Audido core initialised.");

    Ok(CoreHandle {
        ctx,
        _runtime: runtime,
        stream: Some(cpal_stream),
    })
}

fn build_cpal_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    mut consumer: HeapCons<f32>,
    ctx: Arc<CoreContext>,
) -> anyhow::Result<cpal::Stream> {
    use cpal::SampleFormat;

    let ctx_for_err = Arc::clone(&ctx);
    let err_fn = move |err| {
        log::error!("CPAL stream error: {}", err);
        
        match err {
            cpal::StreamError::DeviceNotAvailable => {
                // Signal the runtime to resolve a new host/device and rebuild the graph.
                log::warn!("Audio device disconnected or stream invalidated. Triggering host resolution...");
                ctx_for_err.emit(CoreEvent::DeviceInvalidated);
            }
            _ => {
                // Handle other backend-specific errors if necessary
            }
        }
    };

    let stream = match config.sample_format() {
        SampleFormat::F32 => {
            let stream_config: cpal::StreamConfig = config.clone().into();
            device.build_output_stream(
                &stream_config,
                move |output: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                    fill_output(&mut consumer, output, &ctx);
                },
                err_fn,
                None,
            )?
        }
        SampleFormat::I16 => {
            let stream_config: cpal::StreamConfig = config.clone().into();
            device.build_output_stream(
                &stream_config,
                move |output: &mut [i16], _info: &cpal::OutputCallbackInfo| {
                    let mut tmp = vec![0.0f32; output.len()];
                    fill_output(&mut consumer, &mut tmp, &ctx);
                    for (o, s) in output.iter_mut().zip(tmp.iter()) {
                        *o = cpal::Sample::from_sample(*s);
                    }
                },
                err_fn,
                None,
            )?
        }
        SampleFormat::U16 => {
            let stream_config: cpal::StreamConfig = config.clone().into();
            device.build_output_stream(
                &stream_config,
                move |output: &mut [u16], _info: &cpal::OutputCallbackInfo| {
                    let mut tmp = vec![0.0f32; output.len()];
                    fill_output(&mut consumer, &mut tmp, &ctx);
                    for (o, s) in output.iter_mut().zip(tmp.iter()) {
                        *o = cpal::Sample::from_sample(*s);
                    }
                },
                err_fn,
                None,
            )?
        }
        fmt => anyhow::bail!("Unsupported CPAL sample format: {:?}", fmt),
    };

    Ok(stream)
}

pub fn resolve_host(handle: &mut CoreHandle) -> anyhow::Result<()> {
    log::info!("Resolving new audio host and rebuilding DSP graph...");

    // Safely drop the old invalid stream on the owning thread
    handle.stream = None;

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("No default audio output device found during host resolution")?;

    let config = device
        .default_output_config()
        .context("Failed to get default output config during host resolution")?;

    let ring = HeapRb::<f32>::new(RING_BUFFER_CAPACITY);
    let (producer, consumer) = ring.split();

    {
        let mut ring_lock = handle.ctx.ring_producer.lock().expect("producer poisoned");
        *ring_lock = producer;
    }
    
    handle.ctx.atomics.device_sample_rate.store(config.sample_rate().0, Ordering::Relaxed);

    let new_stream = build_cpal_stream(&device, &config, consumer, Arc::clone(&handle.ctx))?;
    new_stream.play().context("Failed to start recovered CPAL stream")?;

    // Hot-swap the new stream into the handle
    handle.stream = Some(new_stream);
    
    log::info!("Successfully rebuilt CPAL stream and DSP graph.");
    
    Ok(())
}

/// Called by the CPAL audio thread to fill each output buffer.
///
/// Reads available samples from the SPSC ring buffer consumer. If fewer samples
/// are available than requested (underrun), the remainder is zero-filled (silence).
/// Volume scaling is applied here to keep the DSP loop allocation-free.
fn fill_output(
    consumer: &mut HeapCons<f32>,
    output: &mut [f32],
    ctx: &CoreContext,
) {
    if !ctx.atomics.is_playing.load(Ordering::Acquire) {
        output.fill(0.0);
        return;
    }

    let volume = ctx.atomics.get_volume();
    let read = {
        use ringbuf::traits::Consumer;
        consumer.pop_slice(output)
    };

    // Apply volume scaling to the portion we received
    for sample in &mut output[..read] {
        *sample *= volume;
    }

    // Zero-fill any samples we couldn't provide (underrun guard)
    output[read..].fill(0.0);
}

/// Long-running Tokio task that emits `CoreEvent::Position` updates at ~10 Hz
/// and detects natural track end (all samples consumed by DSP feed loop).
async fn run_position_watcher(ctx: Arc<CoreContext>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(100));
    loop {
        interval.tick().await;

        if !ctx.atomics.is_playing.load(Ordering::Relaxed) {
            continue;
        }

        let sample_rate = ctx.atomics.sample_rate.load(Ordering::Relaxed) as f32;
        let channels = ctx.atomics.num_channels.load(Ordering::Relaxed) as f32;
        let pos_samples = ctx.atomics.position_samples.load(Ordering::Relaxed);
        let total_samples = ctx.atomics.total_samples.load(Ordering::Relaxed);

        if sample_rate > 0.0 && channels > 0.0 {
            let frames_per_second = sample_rate * channels;
            let current = pos_samples as f32 / frames_per_second;
            let total = total_samples as f32 / frames_per_second;
            ctx.emit(CoreEvent::Position { current, total });

            // Detect natural track end
            if total_samples > 0 && pos_samples >= total_samples {
                log::info!("Track finished naturally — advancing queue.");
                ctx.atomics.is_playing.store(false, Ordering::Release);

                let next_idx = {
                    let queue = ctx.queue.lock().expect("queue lock poisoned");
                    queue.next_index()
                };

                if let Some(idx) = next_idx {
                    let ctx2 = Arc::clone(&ctx);
                    tokio::spawn(async move {
                        crate::modules::playback::play_queue_index_inner(ctx2, idx).await;
                    });
                } else {
                    ctx.emit(CoreEvent::Stopped);
                }
            }
        }
    }
}