//! Paused system-image cache for the debugger PPU viewer: repeats
//! while paused serve here; memory moves mark dirty.

use std::sync::mpsc;

use nerust_core_traits::{
    EmuCommand,
    debugger::{DebugImage, DebuggerError},
};
use nerust_emu_thread::OperationError;

use super::{EmuCore, recv_debug_reply};

/// Live PPU image batch: generation plus one `(label, width, height, rgba)` tuple per image.
pub type LiveImages = Option<(u64, Vec<(String, u32, u32, Vec<u8>)>)>;

/// Cached system images for the debugger PPU viewer; viewers compare `generation`.
/// Anything advancing or replacing memory marks dirty.
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

/// One stored image decode: the source generation, the pixels, and content-change state.
pub(crate) struct StoredImages {
    pub(crate) generation: u64,
    pub(crate) images: Vec<DebugImage>,
    pub(crate) fresh: bool,
}

/// FNV-1a content hash over decoded image bytes (dependency-free).
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
    /// Mark cached system images stale; the next `debug_images` refetches and bumps generation.
    pub(crate) fn mark_images_dirty(&self) {
        if let Ok(mut cache) = self.image_cache.lock() {
            cache.dirty = true;
            cache.generation = cache.generation.wrapping_add(1);
        }
    }

    /// On-demand system images through the emu thread, cached while paused.
    /// Returns the source generation so viewers skip re-deriving known generations.
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

    /// Paused-cache lookup for [`EmuCore::debug_images`]: a fresh hit skips the emu thread.
    /// Pure query with no side effects; callers mark dirty explicitly when memory may advance.
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

    /// Store a fetched decode and report freshness; a poisoned cache still serves.
    /// Content-identical refetches report the *served* generation so viewers skip re-deriving.
    pub(crate) fn images_store(&self, images: Vec<DebugImage>) -> StoredImages {
        let hash = hash_debug_images(&images);
        let mut cache = self.image_cache.lock().unwrap_or_else(|e| {
            log::warn!("image cache mutex poisoned, recovering");
            e.into_inner()
        });
        cache.images.clone_from(&images);
        cache.dirty = false;
        if cache.served_hash == Some(hash) {
            // Identical content collapses the dirty bump back to the served generation.
            // Same pixels, same generation: viewers skip re-deriving instead of flapping.
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
