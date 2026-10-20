//! Headless dialogs: canned ROM picks with recorded errors.
//! Production code never touches this module.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

/// `DialogPorts` with a canned pick, recording shown errors. Clone
/// shares state like the loader above.
#[derive(Debug, Clone, Default)]
pub(crate) struct FakeDialogs {
    inner: Arc<FakeDialogsInner>,
}

#[derive(Debug, Default)]
struct FakeDialogsInner {
    pick: Mutex<Option<PathBuf>>,
    errors: Mutex<Vec<String>>,
}

impl FakeDialogs {
    pub(crate) fn with_pick(pick: Option<PathBuf>) -> Self {
        let dialogs = Self::default();
        *crate::aux::bridge::lock_recovered(&dialogs.inner.pick) = pick;
        dialogs
    }

    pub(crate) fn errors(&self) -> Vec<String> {
        crate::aux::bridge::lock_recovered(&self.inner.errors).clone()
    }
}

impl crate::ports::DialogPorts for FakeDialogs {
    fn pick_rom_file(
        &self,
        _language: nerust_settings_core::model::language::AppLanguage,
        _filters: Vec<(String, Vec<String>)>,
    ) -> Option<PathBuf> {
        crate::aux::bridge::lock_recovered(&self.inner.pick).clone()
    }

    fn show_load_error(
        &self,
        _language: nerust_settings_core::model::language::AppLanguage,
        detail: &str,
    ) {
        crate::aux::bridge::lock_recovered(&self.inner.errors).push(detail.to_string());
    }

    fn show_error_async(
        &self,
        _language: nerust_settings_core::model::language::AppLanguage,
        detail: String,
    ) -> std::sync::mpsc::Receiver<()> {
        crate::aux::bridge::lock_recovered(&self.inner.errors).push(detail);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let _ = done_tx.send(());
        done_rx
    }
}
