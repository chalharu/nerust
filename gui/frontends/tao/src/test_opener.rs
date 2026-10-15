//! Headless window opener: fails closed while recording requests.
//! Production code never touches this module.

use std::sync::{Arc, Mutex};

/// Recorded window-creation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenerCall {
    Main,
    Settings,
    Debug,
    Ppu,
    Osd,
    History,
}

/// `WindowOpener` that fails every creation closed and records calls.
/// `Some` handles are unbuildable headless (they embed `TaoWindow`),
/// so only the failure/skip paths are reachable — the honest maximum.
#[derive(Debug, Default)]
pub(crate) struct FakeOpener {
    calls: Mutex<Vec<OpenerCall>>,
    main_detail: Mutex<String>,
}

impl FakeOpener {
    pub(crate) fn failing(detail: &str) -> Self {
        let opener = Self::default();
        *crate::aux::bridge::lock_recovered(&opener.main_detail) = detail.to_string();
        opener
    }

    pub(crate) fn calls(&self) -> Vec<OpenerCall> {
        crate::aux::bridge::lock_recovered(&self.calls).clone()
    }
}

impl crate::ports::WindowOpener for FakeOpener {
    fn create_main(
        &self,
        _builder: tao::window::WindowBuilder,
    ) -> Result<Arc<tao::window::Window>, String> {
        crate::aux::bridge::lock_recovered(&self.calls).push(OpenerCall::Main);
        Err(crate::aux::bridge::lock_recovered(&self.main_detail).clone())
    }

    fn create_settings(
        &self,
        _snapshot: nerust_gui_runtime::settings::SettingsSnapshot,
        _registry: Arc<nerust_gui_shell::registry::SystemRegistry>,
        _audio: Arc<nerust_core_traits::audio::AudioBackendRegistry>,
    ) -> Option<crate::aux::settings::window::SettingsWindowHandle> {
        crate::aux::bridge::lock_recovered(&self.calls).push(OpenerCall::Settings);
        None
    }

    fn create_debug(
        &self,
        _display: nerust_gui_viewmodel::debugger::DisplayCache,
        _spaces: Vec<(nerust_core_traits::debugger::SpaceId, String, u32)>,
        _mem_addr: u32,
        _language: nerust_settings_core::model::language::AppLanguage,
        _position: Option<(i32, i32)>,
    ) -> Option<crate::aux::debug::DebugWindowHandle> {
        crate::aux::bridge::lock_recovered(&self.calls).push(OpenerCall::Debug);
        None
    }

    fn create_ppu(
        &self,
        _bridge: &Arc<crate::aux::bridge::DebugBridge>,
        _position: Option<(i32, i32)>,
    ) -> Option<crate::aux::ppu::PpuWindowHandle> {
        crate::aux::bridge::lock_recovered(&self.calls).push(OpenerCall::Ppu);
        None
    }

    fn create_osd(
        &self,
        _position: (i32, i32),
        _size: (f64, f64),
    ) -> Option<crate::aux::osd::OsdWindowHandle> {
        crate::aux::bridge::lock_recovered(&self.calls).push(OpenerCall::Osd);
        None
    }

    fn create_history(
        &self,
        _language: nerust_settings_core::model::language::AppLanguage,
    ) -> Option<crate::aux::osd::HistoryWindowHandle> {
        crate::aux::bridge::lock_recovered(&self.calls).push(OpenerCall::History);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fake_opener_records_ppu_without_building() {
        use crate::aux::bridge::DebugBridge;
        use crate::ports::WindowOpener;
        use nerust_core_traits::debugger::SpaceId;
        use nerust_gui_viewmodel::debugger::DisplayCache;
        use nerust_settings_core::model::language::AppLanguage;
        let opener = FakeOpener::failing("no display");
        let bridge = std::sync::Arc::new(DebugBridge::new(
            DisplayCache::default(),
            vec![(SpaceId(0), "Memory".to_string(), 0)],
            0,
            AppLanguage::English,
        ));
        assert!(opener.create_ppu(&bridge, None).is_none());
        assert!(opener.calls().contains(&OpenerCall::Ppu));
    }
}
