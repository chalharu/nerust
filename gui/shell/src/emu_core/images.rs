//! Paused system-image cache for the debugger PPU viewer.
//!
//! Decode is the drain's dearest pass (GBA re-reads 131k bus bytes per
//! fetch) while paused memory never changes, so repeats serve from
//! here. Split from the transport facade: cache policy lives with the
//! cache, not with every `EmuCore` caller.

use std::sync::mpsc;

use nerust_core_traits::{
    EmuCommand,
    debugger::{DebugImage, DebuggerError},
};
use nerust_emu_thread::OperationError;

use super::{EmuCore, recv_debug_reply};

/// Cached system images (pattern tables) for the debugger PPU viewer.
///
/// Decode is the drain's dearest pass (GBA re-reads 131k bus bytes
/// per fetch) while paused memory never changes, so repeats serve
/// from here. Anything that advances or replaces memory — step,
/// write, load/unload/reset/state import, or a running core —
/// marks the cache dirty instead. `generation` bumps on every dirty mark:
/// PPU viewers compare generations instead of hashing megabytes.
#[derive(Debug, Clone)]
pub(crate) struct CachedImages {
    pub(crate) dirty: bool,
    pub(crate) generation: u64,
    pub(crate) images: Vec<DebugImage>,
    /// Content hash and generation of the last served decode, for the
    /// identical-content skip in [`EmuCore::images_store`].
    pub(crate) served_hash: Option<u64>,
    pub(crate) served_generation: u64,
}

/// One stored image decode: the source generation, the pixels, and
/// whether the content changed since the last serve. Unchanged
/// refetches carry the served generation so viewers skip re-deriving.
pub(crate) struct StoredImages {
    pub(crate) generation: u64,
    pub(crate) images: Vec<DebugImage>,
    pub(crate) fresh: bool,
}

/// FNV-1a content hash over decoded image bytes. ~100ns per 64KB on
/// modern cores: orders of magnitude cheaper than the 512KB RGBA
/// blit it can skip, and dependency-free.
fn hash_debug_images(images: &[DebugImage]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    let mut mix = |bytes: &[u8]| {
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    };
    for image in images {
        mix(image.id.as_bytes());
        mix(&image.width.to_le_bytes());
        mix(&image.height.to_le_bytes());
        for triple in &image.palette {
            mix(triple);
        }
        mix(&image.pixels);
    }
    hash
}

impl EmuCore {
    /// Mark cached system images stale. Called by every path that
    /// advances or replaces memory; the next `debug_images` refetches.
    /// The generation bumps so viewers can compare generations.
    /// `pub(crate)`: the async debug drain shares the same cache
    /// through its cloned handles, so it marks through this too.
    pub(crate) fn mark_images_dirty(&self) {
        if let Ok(mut cache) = self.image_cache.lock() {
            cache.dirty = true;
            cache.generation = cache.generation.wrapping_add(1);
        }
    }

    /// On-demand system images through the emu thread. Same cold-path
    /// contract as [`EmuCore::disassemble`], plus a pause-frozen
    /// cache: repeats while paused serve the stored decode, so page
    /// and refresh drains skip the tile re-read. A running core
    /// advances memory at any time and always refetches. Returns the
    /// source generation with the pixels: viewers skip re-deriving
    /// when the generation they already show comes back.
    pub fn debug_images(
        &self,
    ) -> Result<Result<(u64, Vec<DebugImage>), DebuggerError>, OperationError> {
        if !self.metrics().paused {
            self.mark_images_dirty();
        }
        if let Some(cached) = self.images_cache_lookup() {
            return Ok(Ok(cached));
        }
        let result = recv_debug_reply(self.images_send()?);
        let images = match result {
            Ok(Ok(images)) => images,
            Ok(Err(error)) => return Ok(Err(error)),
            Err(_) => return Err(OperationError::NoReply),
        };
        let stored = self.images_store(images);
        Ok(Ok((stored.generation, stored.images)))
    }

    /// Paused-cache lookup for [`EmuCore::debug_images`]: a fresh hit
    /// serves without touching the emu thread. Pure query, no side
    /// effects: callers mark dirty explicitly when memory may have
    /// advanced (a running core always refetches).
    pub(crate) fn images_cache_lookup(&self) -> Option<(u64, Vec<DebugImage>)> {
        self.image_cache
            .lock()
            .ok()
            .filter(|cache| !cache.dirty && !cache.images.is_empty())
            .map(|cache| (cache.generation, cache.images.clone()))
    }

    /// Send half of the [`EmuCore::debug_images`] fetch: queues without
    /// blocking so the async debug drain can poll on its own tick.
    pub(crate) fn images_send(
        &self,
    ) -> Result<mpsc::Receiver<Result<Vec<DebugImage>, DebuggerError>>, OperationError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.emu
            .send(EmuCommand::DebuggerImages { reply: reply_tx })
            .map_err(|_| OperationError::WorkerUnavailable)?;
        Ok(reply_rx)
    }

    /// Store a fetched decode and report freshness. Content-identical
    /// refetches report the *served* generation so viewers skip
    /// re-deriving; a poisoned cache still serves instead of failing.
    pub(crate) fn images_store(&self, images: Vec<DebugImage>) -> StoredImages {
        let hash = hash_debug_images(&images);
        let mut cache = self.image_cache.lock().unwrap_or_else(|e| {
            log::warn!("image cache mutex poisoned, recovering");
            e.into_inner()
        });
        cache.images.clone_from(&images);
        cache.dirty = false;
        if cache.served_hash == Some(hash) {
            // Identical content: collapse the dirty bump back to the
            // served generation so cache hits agree with stores —
            // otherwise viewers would flap between two generations of
            // the same pixels. Same pixels, same generation, zero UI
            // work, forever.
            cache.generation = cache.served_generation;
            StoredImages {
                generation: cache.served_generation,
                images,
                fresh: false,
            }
        } else {
            cache.served_hash = Some(hash);
            cache.served_generation = cache.generation;
            StoredImages {
                generation: cache.generation,
                images,
                fresh: true,
            }
        }
    }
}
