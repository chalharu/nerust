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
/// canned picks and record errors. `Send + Sync`: error dialogs run on the modal actor so the event loop never stalls behind a modal (see `ModalFlight`).
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
        ModalActor::global().show(language, detail)
    }
}

/// Single modal actor: one queue, one worker thread for all error dialogs.
/// Per-batch spawn-a-thread is gone; dead-queue means a dead ack channel.
struct ModalActor {
    tx: std::sync::mpsc::Sender<(AppLanguage, String, std::sync::mpsc::Sender<()>)>,
}

impl ModalActor {
    /// Process-wide actor: the worker spawns once, on first error.
    fn global() -> &'static Self {
        static ACTOR: std::sync::OnceLock<ModalActor> = std::sync::OnceLock::new();
        ACTOR.get_or_init(|| {
            let (tx, rx) =
                std::sync::mpsc::channel::<(AppLanguage, String, std::sync::mpsc::Sender<()>)>();
            if let Err(error) = std::thread::Builder::new()
                .name("nerust-error-dialog".to_string())
                .spawn(move || {
                    for (language, detail, done) in rx {
                        show_blocking(language, &detail);
                        let _ = done.send(());
                    }
                })
            {
                log::error!("error dialog worker failed to spawn: {error}");
            }
            ModalActor { tx }
        })
    }

    /// Queue one modal: the worker shows it, then acks through the channel.
    /// A dead channel means the worker died; the host reaps it as a failed ack.
    fn show(&self, language: AppLanguage, detail: String) -> std::sync::mpsc::Receiver<()> {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        if self.tx.send((language, detail, done_tx)).is_err() {
            log::error!("error dialog worker gone; ack will fail");
        }
        done_rx
    }
}

/// Blocking modal body: runs only on the actor worker thread.
fn show_blocking(language: AppLanguage, detail: &str) {
    rfd::MessageDialog::new()
        .set_title(text(language, UiText::Error))
        .set_description(detail)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
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
        create_aux_window(self.target, kind, params)
    }
}

/// Direct kind dispatch: the closed `AuxKind` needs no table, and a
/// foreign variant fails closed in the splitter below.
fn create_aux_window(
    target: &EventLoopWindowTarget<UserEvent>,
    kind: AuxKind,
    params: AuxOpenParams,
) -> Option<Box<dyn AuxController>> {
    match kind {
        AuxKind::Settings => {
            let (snapshot, registry, audio) = settings_parts(params)?;
            crate::aux::settings::window::SettingsWindowHandle::new(
                snapshot, registry, audio, target,
            )
            .map(aux_box)
        }
        AuxKind::Debug => {
            let (display, spaces, mem_addr, language, position) = debug_parts(params)?;
            crate::aux::debug::DebugWindowHandle::new(
                display, spaces, mem_addr, language, target, position,
            )
            .map(aux_box)
        }
        AuxKind::Ppu => {
            let (bridge, position) = ppu_parts(params)?;
            crate::aux::ppu::PpuWindowHandle::new(Arc::clone(&bridge), target, position)
                .map(aux_box)
        }
        AuxKind::History => {
            let language = history_parts(params)?;
            crate::aux::osd::HistoryWindowHandle::new(target, language).map(aux_box)
        }
    }
}

/// Box a concrete aux handle behind its capability ports.
fn aux_box<T: AuxController + 'static>(handle: T) -> Box<dyn AuxController> {
    Box::new(handle)
}

/// Split settings inputs: snapshot, registry, and audio backend.
type SettingsParts = (
    SettingsSnapshot,
    Arc<SystemRegistry>,
    Arc<AudioBackendRegistry>,
);

/// Split one settings open: the variant check is pure, so tests cover
/// foreign-variant rejection without a display.
fn settings_parts(params: AuxOpenParams) -> Option<SettingsParts> {
    let AuxOpenParams::Settings {
        snapshot,
        registry,
        audio,
    } = params
    else {
        return None;
    };
    Some((snapshot, registry, audio))
}

/// Split one debugger open: pure variant check, display-free tests.
/// Split debugger inputs: display, spaces, address, language, position.
type DebugParts = (
    DisplayCache,
    Vec<(SpaceId, String, u32)>,
    u32,
    AppLanguage,
    Option<(i32, i32)>,
);

fn debug_parts(params: AuxOpenParams) -> Option<DebugParts> {
    let AuxOpenParams::Debug {
        display,
        spaces,
        mem_addr,
        language,
        position,
    } = params
    else {
        return None;
    };
    Some((display, spaces, mem_addr, language, position))
}

/// Split one viewer open: pure variant check, display-free tests.
type PpuParts = (Arc<DebugBridge>, Option<(i32, i32)>);

fn ppu_parts(params: AuxOpenParams) -> Option<PpuParts> {
    let AuxOpenParams::Ppu { bridge, position } = params else {
        return None;
    };
    Some((bridge, position))
}

/// Split one history open: pure variant check, display-free tests.
fn history_parts(params: AuxOpenParams) -> Option<AppLanguage> {
    let AuxOpenParams::History { language } = params else {
        return None;
    };
    Some(language)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aux::domain::{AuxNativeEvent, NativeWindowId};
    use crate::aux::focus::{AuxEvents, AuxIdentity, AuxRender};

    /// Port owner's contract double: the trait home must not depend
    /// on the shared fake, or the test graph cycles back on itself.
    struct ProbeAux {
        kind: AuxKind,
    }

    impl AuxIdentity for ProbeAux {
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

    impl AuxRender for ProbeAux {
        fn handle_common(&mut self, _event: &AuxNativeEvent) {}
        fn feed_event(&mut self, _event: AuxNativeEvent) -> Option<iced::Event> {
            None
        }
        fn render(&mut self) -> Option<String> {
            None
        }
    }

    impl AuxEvents for ProbeAux {
        fn handle(&mut self, _mapped: iced::Event) -> bool {
            false
        }
    }

    impl AuxController for ProbeAux {}

    #[test]
    fn aux_box_preserves_kind_and_stays_quiet() {
        let boxed = aux_box(ProbeAux {
            kind: AuxKind::Settings,
        });
        assert_eq!(boxed.kind(), AuxKind::Settings);
        assert!(!boxed.close_requested());
    }

    #[test]
    fn modal_actor_is_singleton() {
        // Singleton identity only: enqueueing here would pollute the
        // process-global queue, so plumbing stays untested by unit tests.
        let first = ModalActor::global() as *const _;
        let second = ModalActor::global() as *const _;
        assert_eq!(first, second);
    }

    #[test]
    fn factory_rejects_foreign_variant_without_mismatch_arm() {
        // Kind routing guarantees the variant; a foreign variant is a
        // caller bug answered with `None`, never a logged mismatch.
        fn history_params() -> AuxOpenParams {
            AuxOpenParams::History {
                language: AppLanguage::English,
            }
        }
        assert!(settings_parts(history_params()).is_none());
        assert!(debug_parts(history_params()).is_none());
        assert!(ppu_parts(history_params()).is_none());
        assert_eq!(history_parts(history_params()), Some(AppLanguage::English));
        // Own variants split cleanly.
        let settings = AuxOpenParams::Settings {
            snapshot: SettingsSnapshot {
                shared: Default::default(),
                local: Default::default(),
                app_state: Default::default(),
            },
            registry: std::sync::Arc::new(SystemRegistry::new(Vec::new())),
            audio: std::sync::Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new()),
        };
        assert!(settings_parts(settings).is_some());
        let debug = AuxOpenParams::Debug {
            display: Default::default(),
            spaces: Vec::new(),
            mem_addr: 0,
            language: AppLanguage::English,
            position: None,
        };
        let (display, spaces, mem_addr, language, position) =
            debug_parts(debug).expect("debug splits");
        assert_eq!(
            (mem_addr, language, position),
            (0, AppLanguage::English, None)
        );
        assert!(spaces.is_empty());
        let _ = display;
        let bridge = std::sync::Arc::new(DebugBridge::new(
            Default::default(),
            Vec::new(),
            0,
            AppLanguage::English,
        ));
        let (back_bridge, back_pos) = ppu_parts(AuxOpenParams::Ppu {
            bridge: std::sync::Arc::clone(&bridge),
            position: Some((8, 16)),
        })
        .expect("ppu splits");
        assert!(std::sync::Arc::ptr_eq(&back_bridge, &bridge));
        assert_eq!(back_pos, Some((8, 16)));
    }
}
