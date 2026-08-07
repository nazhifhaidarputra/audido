//! # DSP Module
//!
//! Non-blocking async handler functions for Equalizer control.

use std::sync::Arc;

use crate::{
    commands::RealtimeCommand,
    dsp::eq::{EqPreset, FilterNode},
    modules::core::CoreContext,
};

fn send_rt(ctx: &CoreContext, cmd: RealtimeCommand) {
    if let Some(tx) = ctx.rt_cmd_tx.lock().expect("rt_cmd_tx poisoned").as_ref() {
        let _ = tx.send(cmd);
    }
}

pub fn set_enabled(ctx: Arc<CoreContext>, enabled: bool) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        ctx.eq_enabled.store(enabled, std::sync::atomic::Ordering::Relaxed);
        send_rt(&ctx, RealtimeCommand::SetEqEnabled(enabled));
        log::info!("EQ enabled: {}", enabled);
    });
}

pub fn set_master_gain(ctx: Arc<CoreContext>, gain_db: f32) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        let linear = 10.0f32.powf(gain_db / 20.0);
        ctx.eq_shadow.lock().expect("eq_shadow poisoned").master_gain = linear;
        send_rt(&ctx, RealtimeCommand::SetEqMasterGain(linear));
        log::info!("EQ master gain: {:.2} dB", gain_db);
    });
}

pub fn set_preset(ctx: Arc<CoreContext>, preset: EqPreset) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        {
            let mut eq = ctx.eq_shadow.lock().expect("eq_shadow poisoned");
            eq.preset = preset;
            eq.parameters_changed();
        }
        send_rt(&ctx, RealtimeCommand::SetEqPreset(preset));
        log::info!("EQ preset: {:?}", preset);
    });
}

pub fn set_filters(ctx: Arc<CoreContext>, filters: Vec<FilterNode>) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        {
            let mut eq = ctx.eq_shadow.lock().expect("eq_shadow poisoned");
            eq.filters = filters.clone();
            eq.parameters_changed();
        }
        send_rt(&ctx, RealtimeCommand::SetAllEqFilters(filters));
        log::info!("EQ filters updated.");
    });
}

pub fn reset_eq(ctx: Arc<CoreContext>) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        ctx.eq_shadow.lock().expect("eq_shadow poisoned").reset_parameters();
        send_rt(&ctx, RealtimeCommand::ResetEq);
        log::info!("EQ reset.");
    });
}

pub fn reset_filter_node(ctx: Arc<CoreContext>, index: usize) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        {
            let mut eq = ctx.eq_shadow.lock().expect("eq_shadow poisoned");
            if let Err(e) = eq.reset_filter_node_param(index) {
                log::warn!("Reset filter node {}: {}", index, e);
            }
        }
        send_rt(&ctx, RealtimeCommand::ResetEqFilterNode(index));
        log::info!("EQ filter node {} reset.", index);
    });
}
