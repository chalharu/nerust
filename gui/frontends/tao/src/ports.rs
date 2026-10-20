//! Headless seams for native surfaces: dialogs and window creation. Tao-local leaf (only inner
//! crates, aux handles, and tao types), so both production implementors and headless fakes depend downward. Production impls live here; fakes live in `crate::test_*`.

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
use crate::aux::focus::AuxKind;
use crate::aux::registry::AuxController;
use crate::event::UserEvent;

/// Native file/error dialogs, seamed for headless tests. Production talks to rfd; fakes return
/// canned picks and record errors. `Send + Sync`: error dialogs run on a short-lived thread so the event loop never stalls behind a modal (see `ModalFlight`).
pub(crate) trait DialogPorts: Send + Sync {
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

    fn show_error_async(
        &self,
        language: AppLanguage,
        detail: String,
    ) -> std::sync::mpsc::Receiver<()> {
        // Associated function, not `self`: the thread owns everything it captures.
        fn show_blocking(language: AppLanguage, detail: &str) {
            rfd::MessageDialog::new()
                .set_title(text(language, UiText::Error))
                .set_description(detail)
                .set_level(rfd::MessageLevel::Error)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
        }
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        if std::thread::Builder::new()
            .name("nerust-error-dialog".to_string())
            .spawn(move || {
                show_blocking(language, &detail);
                let _ = done_tx.send(());
            })
            .is_err()
        {
            log::error!("error dialog thread failed to spawn");
        }
        done_rx
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

/// Aux-window open parameters: one variant per capability.
pub(crate) enum AuxOpenParams {
    Settings {
        snapshot: SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio: Arc<AudioBackendRegistry>,
    },
    Debug {
        display: DisplayCache,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
        language: AppLanguage,
        position: Option<(i32, i32)>,
    },
    Ppu {
        bridge: Arc<DebugBridge>,
        position: Option<(i32, i32)>,
    },
    History {
        language: AppLanguage,
    },
}

/// Window creation surface: main plus one capability-routed aux entry.
/// Every method fails closed (`None`/`Err`) on headless fakes.
pub(crate) trait WindowOpener {
    fn create_main(&self, builder: WindowBuilder) -> Result<Arc<TaoWindow>, String>;
    /// Capability-routed aux creation: kind selects the port, params carry the inputs.
    fn create_aux(&self, kind: AuxKind, params: AuxOpenParams) -> Option<Box<dyn AuxController>>;
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

    fn create_aux(&self, kind: AuxKind, params: AuxOpenParams) -> Option<Box<dyn AuxController>> {
        match (kind, params) {
            (
                AuxKind::Settings,
                AuxOpenParams::Settings {
                    snapshot,
                    registry,
                    audio,
                },
            ) => self.create_settings(snapshot, registry, audio).map(aux_box),
            (
                AuxKind::Debug,
                AuxOpenParams::Debug {
                    display,
                    spaces,
                    mem_addr,
                    language,
                    position,
                },
            ) => self
                .create_debug(display, spaces, mem_addr, language, position)
                .map(aux_box),
            (AuxKind::Ppu, AuxOpenParams::Ppu { bridge, position }) => {
                self.create_ppu(&bridge, position).map(aux_box)
            }
            (AuxKind::History, AuxOpenParams::History { language }) => {
                self.create_history(language).map(aux_box)
            }
            _ => {
                log::error!("aux open kind/params mismatch");
                None
            }
        }
    }
}

/// Box a concrete aux handle behind its capability ports.
fn aux_box<T: AuxController + 'static>(handle: T) -> Box<dyn AuxController> {
    Box::new(handle)
}

impl TaoWindowOpener<'_> {
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

    fn create_history(
        &self,
        language: AppLanguage,
    ) -> Option<crate::aux::osd::HistoryWindowHandle> {
        crate::aux::osd::HistoryWindowHandle::new(self.target, language)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aux::focus::{AuxEvents, AuxIdentity, AuxRender};
    use crate::aux::native::{AuxNativeEvent, NativeWindowId};

    struct StubAux {
        kind: AuxKind,
    }

    impl AuxIdentity for StubAux {
        fn kind(&self) -> AuxKind {
            self.kind
        }
        fn native_id(&self) -> NativeWindowId {
            NativeWindowId::dummy()
        }
        fn close_requested(&self) -> bool {
            false
        }
        fn focus_window(&self) {}
        fn request_redraw(&self) {}
    }

    impl AuxRender for StubAux {
        fn handle_common(&mut self, _event: &AuxNativeEvent) {}
        fn feed_event(&mut self, _event: AuxNativeEvent) -> Option<iced::Event> {
            None
        }
        fn render(&mut self) {}
    }

    impl AuxEvents for StubAux {
        fn handle(&mut self, _mapped: iced::Event) -> bool {
            false
        }
    }

    impl crate::aux::registry::DebugDrainPort for StubAux {}
    impl crate::aux::registry::LiveImagesPort for StubAux {}
    impl crate::aux::registry::SettingsClosePort for StubAux {}
    impl crate::aux::registry::HistorySyncPort for StubAux {}
    impl crate::aux::registry::BridgeSharePort for StubAux {}
    impl AuxController for StubAux {}

    /// Local failing opener: records aux requests without a display.
    /// Defined here so these port tests depend only on the port surface.
    struct RecordingOpener {
        calls: std::sync::Mutex<Vec<AuxKind>>,
    }

    impl RecordingOpener {
        fn new() -> Self {
            Self {
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<AuxKind> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl WindowOpener for RecordingOpener {
        fn create_main(&self, _builder: WindowBuilder) -> Result<Arc<TaoWindow>, String> {
            Err("no display".to_string())
        }

        fn create_aux(
            &self,
            kind: AuxKind,
            _params: AuxOpenParams,
        ) -> Option<Box<dyn AuxController>> {
            self.calls.lock().unwrap().push(kind);
            None
        }
    }

    #[test]
    fn aux_open_records_settings_arm() {
        let params = AuxOpenParams::Settings {
            snapshot: SettingsSnapshot {
                shared: Default::default(),
                local: Default::default(),
                app_state: Default::default(),
            },
            registry: std::sync::Arc::new(SystemRegistry::new(Vec::new())),
            audio: std::sync::Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new()),
        };
        let opener = RecordingOpener::new();
        assert!(opener.create_aux(AuxKind::Settings, params).is_none());
        assert_eq!(opener.calls(), [AuxKind::Settings]);
    }

    #[test]
    fn aux_open_records_debug_arm() {
        let opener = RecordingOpener::new();
        assert!(
            opener
                .create_aux(
                    AuxKind::Debug,
                    AuxOpenParams::Debug {
                        display: Default::default(),
                        spaces: Vec::new(),
                        mem_addr: 0,
                        language: AppLanguage::English,
                        position: None,
                    },
                )
                .is_none()
        );
        assert_eq!(opener.calls(), [AuxKind::Debug]);
    }

    #[test]
    fn aux_open_records_ppu_arm() {
        let bridge = std::sync::Arc::new(DebugBridge::new(
            Default::default(),
            Vec::new(),
            0,
            AppLanguage::English,
        ));
        let opener = RecordingOpener::new();
        assert!(
            opener
                .create_aux(
                    AuxKind::Ppu,
                    AuxOpenParams::Ppu {
                        bridge,
                        position: Some((8, 16)),
                    },
                )
                .is_none()
        );
        assert_eq!(opener.calls(), [AuxKind::Ppu]);
    }

    #[test]
    fn aux_open_records_history_arm() {
        let opener = RecordingOpener::new();
        assert!(
            opener
                .create_aux(
                    AuxKind::History,
                    AuxOpenParams::History {
                        language: AppLanguage::English,
                    },
                )
                .is_none()
        );
        assert_eq!(opener.calls(), [AuxKind::History]);
    }

    #[test]
    fn aux_box_preserves_kind_and_stays_quiet() {
        let boxed = aux_box(StubAux {
            kind: AuxKind::Settings,
        });
        assert_eq!(boxed.kind(), AuxKind::Settings);
        assert!(!boxed.close_requested());
    }
}
