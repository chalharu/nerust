//! Android settings view-model core: process-wide dependencies, the
//! per-command view-model build, vault display resolution, and snapshot
//! writeback.
//!
//! Composition root for the settings screens. `wire` keeps pure transport
//! (rows/sections/envelopes); `projection` builds sections; `commands`
//! applies edits; `capture` routes hardware keys. All of them build
//! through here, so lifecycle owns exactly one module.

use std::{
    rc::Rc,
    sync::{Arc, OnceLock},
};

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_viewmodel::settings::{SettingsViewModel, StoragePathError, StoragePathValidator};
use nerust_settings_core::{
    i18n::{FixedLocale, I18n},
    model::language::AppLanguage,
};
use nerust_system_traits::factory::CoreFactory;

/// View-model dependencies shared across JNI calls. `CoreFactory` is
/// `Send + Sync`; the snapshot travels per call, so no editor state is kept.
pub(crate) struct VmDeps {
    pub factories: Vec<Arc<dyn CoreFactory>>,
    pub rates: Arc<[u32]>,
}

static VM_DEPS: OnceLock<VmDeps> = OnceLock::new();

/// Publish the factories and probed sample rates once at startup.
/// Called from the event thread; JNI commands only read. Later installs
/// are ignored: every caller publishes the same registry and rates.
pub(crate) fn install_vm_deps(factories: Vec<Arc<dyn CoreFactory>>, rates: Arc<[u32]>) {
    let _ = VM_DEPS.set(VmDeps { factories, rates });
}

pub(super) fn with_vm_deps<T>(
    operation: impl FnOnce(&VmDeps) -> Result<T, String>,
) -> Result<T, String> {
    let Some(deps) = VM_DEPS.get() else {
        return Err("android settings not initialized".to_string());
    };
    operation(deps)
}

/// Android storage lives behind SAF: the document-tree picker flow owns
/// existence, so this only rejects malformed paths. Snapshot-level URI rules
/// still run in the view-model validator.
#[derive(Debug)]
struct SafStorageValidator;

impl StoragePathValidator for SafStorageValidator {
    fn validate(&self, path: &std::path::Path) -> Result<(), StoragePathError> {
        let text = path.to_string_lossy();
        if text.is_empty() || text.contains('\0') {
            return Err(StoragePathError::Inaccessible(
                "storage path must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

pub(super) fn build_vm(
    snapshot: &SettingsSnapshot,
    deps: &VmDeps,
    i18n: I18n,
) -> Result<SettingsViewModel, String> {
    // Display copy only: the VM resolves every label through the snapshot
    // language, so SystemDefault is concretized here (the OS query sees
    // no locale on Android). The draft keeps the user's choice;
    // writebacks restore it via `sync_snapshot_from_vm`.
    let mut resolved = snapshot.clone();
    resolved.shared.general.language = i18n.language();
    SettingsViewModel::new(
        resolved,
        deps.factories.clone(),
        Arc::clone(&deps.rates),
        Rc::new(SafStorageValidator),
        AppLanguage::English,
    )
    .map_err(|error| error.to_string())
}

/// Display language for one projection: an explicit draft choice wins,
/// SystemDefault follows the vault-pushed terminal `Locale` (`None`
/// until pushed keeps the choice for the host fallback).
pub(crate) fn resolve_display(setting: AppLanguage, system: Option<AppLanguage>) -> I18n {
    I18n::resolve_with(setting, FixedLocale(system))
}

/// Supported set back to a BCP-47 tag for the save push (empty means
/// the terminal locale). Inverse of the shared tag→language rule.
pub(crate) fn app_language_tag(language: AppLanguage) -> &'static str {
    match language {
        AppLanguage::Japanese => "ja",
        AppLanguage::English => "en",
        AppLanguage::SystemDefault => "",
    }
}

/// Display copies concretize SystemDefault, so a resolved language readback
/// is ambiguous: restore the draft choice when the value matches the
/// resolved locale (untouched), keeping a genuine explicit change.
pub(super) fn pin_draft_language(
    draft: AppLanguage,
    resolved: AppLanguage,
    target: &mut AppLanguage,
) {
    if *target == resolved {
        *target = draft;
    }
}

/// Write the VM working copy back, keeping the draft language choice
/// (the VM works on the concretized display copy).
pub(super) fn sync_snapshot_from_vm(
    snapshot: &mut SettingsSnapshot,
    vm: &SettingsViewModel,
    i18n: I18n,
) {
    let original = snapshot.shared.general.language;
    let updated = vm.snapshot();
    *snapshot = updated;
    pin_draft_language(
        original,
        i18n.language(),
        &mut snapshot.shared.general.language,
    );
}
