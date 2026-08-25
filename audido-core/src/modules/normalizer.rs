//! # Normalizer Module

use std::sync::Arc;

use crate::{
    commands::RealtimeCommand, dsp::normalization::NormalizationMode, modules::core::CoreContext,
};

#[derive(Debug, Clone, Copy)]
pub struct NormalizerMeter {
    pub current_gain_db: f32,
    pub measured_level: f32,
}

pub fn meter(ctx: &CoreContext) -> NormalizerMeter {
    let normalizer = ctx
        .normalizer_shadow
        .lock()
        .expect("normalizer_shadow poisoned");
    NormalizerMeter {
        current_gain_db: normalizer.current_gain_db(),
        measured_level: normalizer.last_measured_level(),
    }
}

fn send_rt(ctx: &CoreContext, cmd: RealtimeCommand) {
    if let Some(tx) = ctx.rt_cmd_tx.lock().expect("rt_cmd_tx poisoned").as_ref() {
        let _ = tx.send(cmd);
    }
}

pub fn set_enabled(ctx: Arc<CoreContext>, enabled: bool) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        ctx.normalizer_enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
        send_rt(&ctx, RealtimeCommand::SetNormalizerEnabled(enabled));
        log::info!("Normalizer enabled: {}", enabled);
    });
}

pub fn set_mode(ctx: Arc<CoreContext>, mode: NormalizationMode) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        ctx.normalizer_shadow
            .lock()
            .expect("normalizer_shadow poisoned")
            .set_mode(mode);
        send_rt(&ctx, RealtimeCommand::SetNormalizerMode(mode));
        log::info!("Normalizer mode: {:?}", mode);
    });
}

pub fn set_target_level(ctx: Arc<CoreContext>, level: f32) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        ctx.normalizer_shadow
            .lock()
            .expect("normalizer_shadow poisoned")
            .set_target_level(level);
        send_rt(&ctx, RealtimeCommand::SetNormalizerTargetLevel(level));
        log::info!("Normalizer target: {:.2}", level);
    });
}

pub fn set_headroom(ctx: Arc<CoreContext>, headroom_db: f32) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        ctx.normalizer_shadow
            .lock()
            .expect("normalizer_shadow poisoned")
            .set_headroom(headroom_db);
        send_rt(&ctx, RealtimeCommand::SetNormalizerHeadroom(headroom_db));
        log::info!("Normalizer headroom: {:.2} dB", headroom_db);
    });
}
