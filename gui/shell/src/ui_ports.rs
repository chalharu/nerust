//! UI seams: toolkit-neutral trait shapes and snapshot structs for
//! frontend chrome. Frontends implement these with their toolkit;
//! `tao` owns the production impls today, `gtk` adopts them piece by piece.

use std::{path::PathBuf, sync::Arc};

use nerust_gui_runtime::debugger::{DebugImageRgba, DisplayCache};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_persistence::model::StateSlotSummary;
use nerust_settings_core::model::{input::ShortcutBinding, language::AppLanguage};

use crate::{
    debug::DebugAction,
    session::osd::HistoryLine,
};

/// Debug-drain port: batch take plus apply for one drain.
pub trait DebugDrainPort {
    /// Debugger batch take (debug only).
    fn take_debug_requests(&mut self) -> Option<(bool, Vec<DebugAction>)>;

    /// Apply a finished drain (debug only).
    fn apply_drain(&mut self, display: DisplayCache, reselected: Option<(u32, Option<u8>)>);
}

/// Live-images port: finished live fetches land here.
pub trait LiveImagesPort {
    /// Apply finished live images (debug only).
    fn apply_live(
        &mut self,
        _generation: u64,
        _images: Vec<DebugImageRgba>,
        _running: bool,
    ) -> bool;
}

/// Settings-close port: pending apply staged for close teardown.
pub trait SettingsClosePort {
    /// Take the pending settings apply on close (settings only).
    fn take_pending_settings(&mut self) -> Option<SettingsSnapshot>;
}

/// History-sync port: content push for the history viewer.
pub trait HistorySyncPort {
    /// Push fresh history content (history only).
    fn sync_history(&mut self, lines: Arc<[HistoryLine]>, language: AppLanguage);
    /// Take a history clear request; the default pump routes here.
    fn take_history_clear(&mut self) -> bool;
}

/// Dialog seam: blocking pick plus sync/async error presentation.
/// Production `rfd` impls live in the frontend; headless fakes in tests.
pub trait DialogPorts: Send + Sync {
    fn pick_rom_file(
        &self,
        language: AppLanguage,
        filters: Vec<(String, Vec<String>)>,
    ) -> Option<PathBuf>;
    fn show_load_error(&self, language: AppLanguage, detail: &str);
    /// Non-blocking error dialog: spawns the modal, returns its ack channel.
    /// A dead channel means the spawn failed; the host reaps it as a failed ack.
    fn show_error_async(
        &self,
        language: AppLanguage,
        detail: String,
    ) -> std::sync::mpsc::Receiver<()>;
}

/// One snapshot for a menu sync: a struct (not a long argument
/// list) so adding a field never trips the arity lint.
pub struct MenuSync<'a> {
    pub loaded: bool,
    pub paused: bool,
    pub slots: &'a [StateSlotSummary],
    pub active_slot: Option<u64>,
    pub settings_open: bool,
    pub language: AppLanguage,
    pub shortcuts: &'a [ShortcutBinding],
}
