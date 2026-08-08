//! # Queue Module

use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::{
    commands::CoreEvent,
    modules::core::CoreContext,
    modules::playback::play_queue_index_inner,
    queue::LoopMode,
};

pub async fn add_to_queue(ctx: Arc<CoreContext>, paths: Vec<String>) {
    let was_empty;
    let is_playing = ctx.atomics.is_playing.load(std::sync::atomic::Ordering::Relaxed);

    {
        let mut queue = ctx.queue.lock().expect("queue poisoned");
        was_empty = queue.items.is_empty();
        let path_bufs: Vec<std::path::PathBuf> = paths.into_iter().map(Into::into).collect();
        queue.add(path_bufs);
    }

    emit_queue_update(&ctx);

    if was_empty && !is_playing {
        play_queue_index_inner(Arc::clone(&ctx), 0).await;
    }
    log::info!("Items added to queue.");
}
pub fn remove_from_queue(ctx: Arc<CoreContext>, id: usize) -> JoinHandle<()>  {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        {
            let mut queue = ctx.queue.lock().expect("queue poisoned");
            queue.remove(id);
        }
        emit_queue_update(&ctx);
        log::info!("Removed queue item id={}", id);
    })
} 

pub async fn clear_queue(ctx: Arc<CoreContext>) {
    crate::modules::playback::stop_inner(&ctx).await;

    {
        let mut queue = ctx.queue.lock().expect("queue poisoned");
        queue.clear();
    }

    *ctx.current_audio.lock().expect("current_audio poisoned") = None;
    ctx.atomics.total_samples.store(0, std::sync::atomic::Ordering::Release);
    ctx.atomics.position_samples.store(0, std::sync::atomic::Ordering::Release);

    emit_queue_update(&ctx);
    ctx.emit(CoreEvent::Stopped);
    log::info!("Queue cleared.");
}

pub fn play_index(ctx: Arc<CoreContext>, index: usize) -> JoinHandle<()>  {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        play_queue_index_inner(ctx, index).await;
    })
}

pub fn next(ctx: Arc<CoreContext>) -> JoinHandle<()> {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        let next_idx = {
            let queue = ctx.queue.lock().expect("queue poisoned");
            queue.next_index()
        };
        match next_idx {
            Some(idx) => play_queue_index_inner(ctx, idx).await,
            None => log::info!("No next track."),
        }
    })
}

pub fn previous(ctx: Arc<CoreContext>) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        let prev_idx = {
            let queue = ctx.queue.lock().expect("queue poisoned");
            queue.prev_index()
        };
        match prev_idx {
            Some(idx) => play_queue_index_inner(ctx, idx).await,
            None => log::info!("No previous track."),
        }
    });
}

pub fn set_loop_mode(ctx: Arc<CoreContext>, mode: LoopMode) {
    let handle = ctx.tokio_handle.clone();
    handle.spawn(async move {
        {
            let mut queue = ctx.queue.lock().expect("queue poisoned");
            queue.loop_mode = mode;
            if mode == LoopMode::Shuffle {
                queue.reshuffle();
            }
        }
        ctx.emit(CoreEvent::LoopModeChanged(mode));
        log::info!("Loop mode: {:?}", mode);
    });
}

pub(crate) fn emit_queue_update(ctx: &CoreContext) {
    let items = {
        let queue = ctx.queue.lock().expect("queue poisoned");
        queue.items.clone()
    };
    ctx.emit(CoreEvent::QueueUpdated(items));
}
