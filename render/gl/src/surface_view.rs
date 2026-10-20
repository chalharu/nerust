//! Surface memo plus teardown accounting for the GL view.
//! Owns the resize/viewport memo and the leaked-name ledger; frame and
//! OSD views borrow the memo instead of issuing per-frame `Get` calls.

use nerust_render_traits::ReservedStrip;

/// One GL name that failed to delete on teardown.
/// Variants name the namespace; the count, not the value, is the signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeakedName {
    Texture(u32),
    Program(u32),
    Array(u32),
    Buffer(u32),
}

/// Teardown ledger: every live name that missed deletion.
/// Best-effort teardown warns but never fails the rebuild that called it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CloseReport {
    pub leaked: Vec<LeakedName>,
}

impl CloseReport {
    pub fn leak(&mut self, name: LeakedName) {
        self.leaked.push(name);
    }

    pub fn is_ok(&self) -> bool {
        self.leaked.is_empty()
    }

    /// Coarse total for warn-once logs.
    pub fn failed_names(&self) -> u32 {
        u32::try_from(self.leaked.len()).unwrap_or(u32::MAX)
    }

    pub fn failed_textures(&self) -> u32 {
        self.count(|name| matches!(name, LeakedName::Texture(_)))
    }

    pub fn failed_programs(&self) -> u32 {
        self.count(|name| matches!(name, LeakedName::Program(_)))
    }

    pub fn failed_arrays(&self) -> u32 {
        self.count(|name| matches!(name, LeakedName::Array(_)))
    }

    pub fn failed_buffers(&self) -> u32 {
        self.count(|name| matches!(name, LeakedName::Buffer(_)))
    }

    fn count(&self, mut pred: impl FnMut(LeakedName) -> bool) -> u32 {
        u32::try_from(self.leaked.iter().copied().filter(|n| pred(*n)).count()).unwrap_or(u32::MAX)
    }
}

/// Surface memo: last resize inputs plus the applied game viewport.
/// Skips redundant GL state writes and the per-frame `GetIntegerv`.
#[derive(Debug, Default)]
pub struct SurfaceView {
    last_resize: Option<(i32, i32, ReservedStrip)>,
    game_viewport: Option<[i32; 4]>,
}

impl SurfaceView {
    pub fn new() -> Self {
        Self::default()
    }

    /// Memoized game viewport for the OSD composite guard.
    /// `Some` skips the per-frame `GetIntegerv`; `None` reads once.
    pub fn game_viewport(&self) -> Option<[i32; 4]> {
        self.game_viewport
    }

    pub fn set_game_viewport(&mut self, viewport: [i32; 4]) {
        self.game_viewport = Some(viewport);
    }

    /// Resize memo hit: identical inputs skip all GL work.
    pub fn resize_hit(&self, w: i32, h: i32, strip: ReservedStrip) -> bool {
        self.last_resize == Some((w, h, strip))
    }

    pub fn record_resize(&mut self, w: i32, h: i32, strip: ReservedStrip, game_box: [i32; 4]) {
        self.last_resize = Some((w, h, strip));
        self.game_viewport = Some(game_box);
    }

    pub fn reset(&mut self) {
        self.last_resize = None;
        self.game_viewport = None;
    }
}
