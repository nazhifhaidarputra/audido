//! # Playback Module
//!
//! Provides non-blocking async handler functions for playback control.
//! All functions take `Arc<CoreContext>` and spawn background Tokio tasks internally
//! so the TUI event loop is **never blocked** — even when loading an audio file from disk.

use std::sync::{Arc, atomic::Ordering};

use crossbeam_channel::unbounded;
use ringbuf::traits::Producer;

use crate::{
    commands::{CoreEvent, RealtimeCommand},
    dsp::eq::Equalizer,
    modules::core::{CHUNK_SIZE, CoreContext},
};

// ==============================================
// =============== Public API  ==================
// ==============================================

/// Resume or start playback. No-op if already playing.
pub async fn play(ctx: Arc<CoreContext>) {
    let has_audio = ctx.current_audio.lock().unwrap().is_some();
    if !has_audio {
        ctx.emit(CoreEvent::Error("No audio loaded".into()));
        return;
    }
    if ctx.atomics.is_playing.load(Ordering::Relaxed) {
        return;
    }
    ctx.atomics.is_playing.store(true, Ordering::Release);
    ctx.emit(CoreEvent::Playing);
    log::info!("Playback resumed.");
}

/// Pause playback. No-op if already paused.
pub async fn pause(ctx: Arc<CoreContext>) {
    if !ctx.atomics.is_playing.load(Ordering::Relaxed) {
        return;
    }
    ctx.atomics.is_playing.store(false, Ordering::Release);
    ctx.emit(CoreEvent::Paused);
    log::info!("Playback paused.");
}

/// Stop playback and reset position to the beginning.
pub fn stop(ctx: Arc<CoreContext>) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        stop_inner(&ctx).await;
        ctx.emit(CoreEvent::Stopped);
        log::info!("Playback stopped.");
    });
}

/// Seek to an absolute position in seconds.
pub fn seek(ctx: Arc<CoreContext>, seconds: f32) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        let sample_rate = ctx.atomics.sample_rate.load(Ordering::Relaxed) as f32;
        let channels = ctx.atomics.num_channels.load(Ordering::Relaxed) as usize;
        let total = ctx.atomics.total_samples.load(Ordering::Relaxed);

        let requested_sample = ((seconds.max(0.0) * sample_rate) as usize * channels).min(total);
        let buffered = ctx
            .current_audio
            .lock()
            .expect("current_audio poisoned")
            .as_ref()
            .map_or(0, |audio| audio.buffered_samples());
        let mut target_sample = requested_sample.min(buffered);
        if channels > 0 {
            target_sample -= target_sample % channels;
        }
        ctx.atomics
            .position_samples
            .store(target_sample, Ordering::Release);
        // Discard samples prefetched before this seek. The DSP loop will refill
        // the output ring beginning at the new retained-buffer cursor.
        ctx.atomics.clear_buffer.store(true, Ordering::Release);

        if let Some(tx) = ctx.rt_cmd_tx.lock().unwrap().as_ref() {
            let _ = tx.send(RealtimeCommand::SeekToFrame(target_sample));
        }
        log::info!(
            "Seeked to {:.2}s (sample {}, requested sample {}, buffered {})",
            target_sample as f32 / (sample_rate * channels.max(1) as f32),
            target_sample,
            requested_sample,
            buffered
        );
    });
}

/// Set output volume. Clamped to [0.0, 1.0].
pub fn set_volume(ctx: Arc<CoreContext>, volume: f32) {
    ctx.atomics.set_volume(volume.clamp(0.0, 1.0));
}

/// Set playback speed multiplier. Clamped to [0.1, 4.0].
pub fn set_speed(ctx: Arc<CoreContext>, speed: f32) {
    ctx.atomics.set_speed(speed.clamp(0.1, 4.0));
}

// ==========================================
// ============ Internal helpers ============
// ==========================================

/// Stop playback, reset sample position to zero, and signal the DSP task to exit.
pub(crate) async fn stop_inner(ctx: &CoreContext) {
    ctx.atomics.is_playing.store(false, Ordering::Release);
    ctx.atomics.position_samples.store(0, Ordering::Release);
    ctx.atomics.clear_buffer.store(true, Ordering::Release);
    let old_tx = ctx.rt_cmd_tx.lock().unwrap().take();
    if let Some(tx) = old_tx {
        let _ = tx.send(RealtimeCommand::Stop);
    }
}

/// Load and start playing the queue item at `index`.
pub(crate) async fn play_queue_index_inner(ctx: Arc<CoreContext>, index: usize) {
    // Snapshot the source variant while holding the queue lock briefly.
    let source = {
        let queue = ctx.queue.lock().expect("queue poisoned");
        match queue.get(index) {
            Some(item) => item.source.clone(),
            None => {
                ctx.emit(CoreEvent::Error(format!("Invalid queue index: {}", index)));
                return;
            }
        }
    };

    log::info!("Loading queue index {} — {:?}", index, source);
    stop_inner(&ctx).await;

    let device_sample_rate = ctx.atomics.device_sample_rate.load(Ordering::Relaxed);

    let audio_data = match source {
        crate::source::AudioSource::Local { path } => {
            let path_str = path.to_string_lossy().to_string();
            match tokio::task::spawn_blocking(move || {
                crate::source::AudioPlaybackData::load_local_audio(
                    &path_str,
                    Some(device_sample_rate),
                )
            })
            .await
            {
                Ok(Ok(data)) => data,
                Ok(Err(e)) => {
                    ctx.emit(CoreEvent::Error(format!("Failed to load audio: {}", e)));
                    return;
                }
                Err(e) => {
                    ctx.emit(CoreEvent::Error(format!("Load task panicked: {}", e)));
                    return;
                }
            }
        }
        crate::source::AudioSource::Youtube { url } => {
            match ctx
                .yt
                .load_youtube_stream(&url, Some(device_sample_rate))
                .await
            {
                Ok(data) => data,
                Err(e) => {
                    ctx.emit(CoreEvent::Error(format!(
                        "Failed to load YouTube stream: {}",
                        e
                    )));
                    return;
                }
            }
        }
    };

    let metadata = audio_data.metadata();
    ctx.atomics
        .sample_rate
        .store(metadata.sample_rate, Ordering::Release);
    ctx.atomics
        .num_channels
        .store(metadata.num_channels as u32, Ordering::Release);
    ctx.atomics
        .total_samples
        .store(audio_data.total_samples(), Ordering::Release);
    ctx.atomics.position_samples.store(0, Ordering::Release);

    {
        let mut queue = ctx.queue.lock().expect("queue poisoned");
        let item_id = queue.get(index).map(|i| i.id);
        if let Some(id) = item_id {
            queue.set_metadata(id, metadata.clone());
        }
        queue.current_index = Some(index);
    }

    {
        let mut eq = ctx.eq_shadow.lock().expect("eq_shadow poisoned");
        let prev_filters = eq.filters.clone();
        let prev_gain = eq.master_gain;
        let prev_preset = eq.preset;
        *eq = Equalizer::new(metadata.sample_rate, metadata.num_channels);
        eq.filters = prev_filters;
        eq.master_gain = prev_gain;
        eq.preset = prev_preset;
        eq.parameters_changed();
    }

    *ctx.current_audio.lock().expect("current_audio poisoned") = Some(audio_data);

    ctx.emit(CoreEvent::TrackChanged {
        index,
        metadata: metadata.clone(),
    });
    ctx.emit(CoreEvent::Loaded(metadata));

    let (rt_tx, rt_rx) = unbounded::<RealtimeCommand>();
    *ctx.rt_cmd_tx.lock().unwrap() = Some(rt_tx);

    let ctx_for_dsp = Arc::clone(&ctx);
    tokio::task::spawn_blocking(move || {
        run_dsp_feed_loop(ctx_for_dsp, rt_rx);
    });

    ctx.atomics.is_playing.store(true, Ordering::Release);
    ctx.emit(CoreEvent::Playing);
}

fn run_dsp_feed_loop(ctx: Arc<CoreContext>, rt_rx: crossbeam_channel::Receiver<RealtimeCommand>) {
    use crate::dsp::dsp_graph::DspNode;

    log::debug!("DSP feed loop started.");

    let mut eq_node = {
        let eq = ctx.eq_shadow.lock().expect("eq poisoned");
        DspNode::new_with_state(eq.clone(), ctx.eq_enabled.load(Ordering::Relaxed))
    };
    let mut norm_node = {
        let norm = ctx.normalizer_shadow.lock().expect("normalizer poisoned");
        DspNode::new_with_state(norm.clone(), ctx.normalizer_enabled.load(Ordering::Relaxed))
    };

    let mut chunk = vec![0.0f32; CHUNK_SIZE];

    loop {
        while let Ok(cmd) = rt_rx.try_recv() {
            match cmd {
                RealtimeCommand::Stop => {
                    log::debug!("DSP loop: Stop received.");
                    return;
                }
                RealtimeCommand::SeekToFrame(frame) => {
                    ctx.atomics.position_samples.store(frame, Ordering::Release);
                }
                RealtimeCommand::UpdateEqFilter(idx, filter) => {
                    eq_node.set_filter(idx, filter);
                }
                RealtimeCommand::SetAllEqFilters(filters) => {
                    eq_node.set_all_filters(filters);
                }
                RealtimeCommand::SetEqMasterGain(gain) => {
                    eq_node.set_master_gain(gain);
                }
                RealtimeCommand::SetEqPreset(preset) => {
                    eq_node.instance.update_preset(preset);
                }
                RealtimeCommand::ResetEq => {
                    eq_node.instance.reset_parameters();
                }
                RealtimeCommand::ResetEqFilterNode(idx) => {
                    let _ = eq_node.instance.reset_filter_node_param(idx);
                }
                RealtimeCommand::SetEqEnabled(on) => {
                    eq_node.on = on;
                }
                RealtimeCommand::SetNormalizerMode(mode) => {
                    norm_node.instance.set_mode(mode);
                }
                RealtimeCommand::SetNormalizerTargetLevel(level) => {
                    norm_node.instance.set_target_level(level);
                }
                RealtimeCommand::SetNormalizerHeadroom(hr) => {
                    norm_node.instance.set_headroom(hr);
                }
                RealtimeCommand::SetNormalizerEnabled(on) => {
                    norm_node.on = on;
                }
            }
        }

        // Acquire the buffer variant — we clone the enum (cheap for both variants)
        // so we don't hold the mutex across the blocking recv below.
        let buffer = {
            let audio = ctx.current_audio.lock().expect("current_audio poisoned");
            match audio.as_ref() {
                Some(d) => d.buffer().clone(),
                None => break,
            }
        };

        let pos = ctx.atomics.position_samples.load(Ordering::Relaxed);

        chunk.clear();
        match &buffer {
            crate::source::AudioBuffer::InMemory(samples) => {
                // Fast path: the full decoded buffer is already in memory.
                if pos >= samples.len() {
                    break; // end of track
                }
                let end = (pos + CHUNK_SIZE).min(samples.len());
                chunk.extend_from_slice(&samples[pos..end]);
            }
            crate::source::AudioBuffer::Stream(samples) => {
                let channels = ctx.atomics.num_channels.load(Ordering::Relaxed) as usize;
                let available = samples.buffered_samples().saturating_sub(pos);
                let complete_frames = available
                    .checked_div(channels)
                    .map_or(available, |frames| frames * channels);
                samples.copy_from(pos, CHUNK_SIZE.min(complete_frames), &mut chunk);

                if chunk.is_empty() {
                    if samples.is_complete() {
                        log::debug!("DSP loop: retained stream exhausted — end of track.");
                        ctx.atomics.position_samples.store(
                            ctx.atomics.total_samples.load(Ordering::Relaxed),
                            Ordering::Release,
                        );
                        break;
                    }

                    // Playback has caught the decoder. Keep the current cursor
                    // stable until another retained batch becomes available.
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                }
            }
        }

        ctx.atomics
            .position_samples
            .store(pos + chunk.len(), Ordering::Release);

        if eq_node.on {
            eq_node.instance.process_frame(&mut chunk);
        }
        if norm_node.on {
            norm_node.instance.process(&mut chunk);
        }

        // Spectrum analysis — push bins into the SPSC ring for the TUI to consume.
        // Use try_lock so we never block the real-time DSP thread.
        if let Ok(mut fft) = ctx.zero_copy_fft.try_lock() {
            let bin_size = ctx.spectrum_bin_size.load(Ordering::Relaxed);
            // Lazy resize: update engine if the TUI changed bin_size.
            if fft.bin_size != bin_size && bin_size > 0 {
                fft.bin_size = bin_size;
            }
            let channels = ctx.atomics.num_channels.load(Ordering::Relaxed) as u16;
            let bins = fft.process(&chunk, channels);
            // Non-blocking push: if the ring is full the TUI is just slow — drop the frame.
            if let Ok(mut prod) = ctx.spectrum_producer.try_lock() {
                let _ = prod.push_slice(&bins);
            }
        }

        // Push to ring buffer (yield on full)
        let mut written = 0;
        while written < chunk.len() {
            let n = {
                let mut prod = ctx.ring_producer.lock().expect("ring_producer poisoned");
                prod.push_slice(&chunk[written..])
            };
            written += n;
            if written < chunk.len() {
                std::thread::sleep(std::time::Duration::from_micros(100));
            }
        }
    }

    log::debug!("DSP feed loop ended.");
}
