//! Headless seams for native surfaces: dialogs and window creation.
//! Tao-local leaf (only inner crates, aux handles, and tao types), so
//! both production implementors and headless fakes depend downward.
//! Production impls live here; fakes live in `crate::test_*`.

use std::{path::PathBuf, sync::Arc};

use tao::{
    event_loop::EventLoopWindowTarget,
    window::{Window as TaoWindow, WindowBuilder},
};

use nerust_core_traits::{audio::AudioBackendRegistry, debugger::SpaceId};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::registry::SystemRegistry;
use nerust_gui_viewmodel::debugger::DisplayCache;
use nerust_settings_core::{
    i18n::{UiText, text},
    model::language::AppLanguage,
};
use rfd::FileDialog;

use crate::aux::bridge::DebugBridge;
use crate::event::UserEvent;

/// Native file/error dialogs, seamed for headless tests. Production
/// talks to rfd; fakes return canned picks and record errors.
pub(crate) trait DialogPorts {
    fn pick_rom_file(
        &self,
        language: AppLanguage,
        filters: Vec<(String, Vec<String>)>,
    ) -> Option<PathBuf>;
    fn show_load_error(&self, language: AppLanguage, detail: &str);
}

/// Production dialogs over rfd.
pub(crate) struct NativeDialogs;

impl DialogPorts for NativeDialogs {
    fn pick_rom_file(
        &self,
        language: AppLanguage,
        filters: Vec<(String, Vec<String>)>,
    ) -> Option<PathBuf> {
        let mut dialog = FileDialog::new().set_title(text(language, UiText::Open));
        for (name, extensions) in filters {
            dialog = dialog.add_filter(name, extensions.as_slice());
        }
        dialog.pick_file()
    }

    fn show_load_error(&self, language: AppLanguage, detail: &str) {
        show_rom_load_error(language, detail);
    }
}

/// Surface ROM load failures in a modal dialog: the previous
/// warn-only path left the empty window with no explanation.
pub(crate) fn show_rom_load_error(language: AppLanguage, detail: &str) {
    let title = text(language, UiText::RomLoadFailed);
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(detail)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

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
