//! Aux/main window creation behind a seam: production builds real
//! tao windows, headless tests record the request and fail closed.
//! Callers take `&dyn WindowOpener` so no `EventLoopWindowTarget`
//! ever reaches test code.

use std::sync::Arc;

use tao::{
    event_loop::EventLoopWindowTarget,
    window::{Window as TaoWindow, WindowBuilder},
};

use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_core_traits::debugger::SpaceId;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::registry::SystemRegistry;
use nerust_gui_viewmodel::debugger::DisplayCache;
use nerust_settings_core::model::language::AppLanguage;

use crate::aux::bridge::DebugBridge;
use crate::event::UserEvent;

/// Window creation surface. Every method fails closed (`None`/`Err`)
/// on headless fakes; production forwards to the handle constructors.
pub(crate) trait WindowOpener {
    fn create_main(&self, builder: WindowBuilder) -> Result<Arc<TaoWindow>, String>;
    fn create_settings(
        &self,
        snapshot: SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio: Arc<AudioBackendRegistry>,
    ) -> Option<crate::aux::settings::window::SettingsWindowHandle>;
    fn create_debug(
        &self,
        display: DisplayCache,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
        language: AppLanguage,
        position: Option<(i32, i32)>,
    ) -> Option<crate::aux::debug::DebugWindowHandle>;
    fn create_ppu(
        &self,
        bridge: &Arc<DebugBridge>,
        position: Option<(i32, i32)>,
    ) -> Option<crate::aux::ppu::PpuWindowHandle>;
}

/// Production opener: real tao windows on the live event loop.
pub(crate) struct TaoWindowOpener<'a> {
    target: &'a EventLoopWindowTarget<UserEvent>,
}

impl<'a> TaoWindowOpener<'a> {
    pub(crate) fn new(target: &'a EventLoopWindowTarget<UserEvent>) -> Self {
        Self { target }
    }
}

impl WindowOpener for TaoWindowOpener<'_> {
    fn create_main(&self, builder: WindowBuilder) -> Result<Arc<TaoWindow>, String> {
        builder
            .build(self.target)
            .map(Arc::new)
            .map_err(|e| e.to_string())
    }

    fn create_settings(
        &self,
        snapshot: SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio: Arc<AudioBackendRegistry>,
    ) -> Option<crate::aux::settings::window::SettingsWindowHandle> {
        crate::aux::settings::window::SettingsWindowHandle::new(
            snapshot,
            registry,
            audio,
            self.target,
        )
    }

    fn create_debug(
        &self,
        display: DisplayCache,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
        language: AppLanguage,
        position: Option<(i32, i32)>,
    ) -> Option<crate::aux::debug::DebugWindowHandle> {
        crate::aux::debug::DebugWindowHandle::new(
            display,
            spaces,
            mem_addr,
            language,
            self.target,
            position,
        )
    }

    fn create_ppu(
        &self,
        bridge: &Arc<DebugBridge>,
        position: Option<(i32, i32)>,
    ) -> Option<crate::aux::ppu::PpuWindowHandle> {
        crate::aux::ppu::PpuWindowHandle::new(Arc::clone(bridge), self.target, position)
    }
}
