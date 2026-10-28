//! Headless seams for native surfaces: dialogs and window creation. Tao-local leaf (only inner
//! crates, aux handles, and tao types), so both production implementors and headless fakes depend downward. Production impls live here; fakes live in `crate::test_*`.

use std::{path::PathBuf, sync::Arc};

use tao::{
    event_loop::EventLoopWindowTarget,
    window::{Window as TaoWindow, WindowBuilder},
};

use nerust_core_traits::debugger::SpaceId;
use nerust_frontend_host::{
    control::ControlScreen,
    widget_session::WidgetSession,
    window::{DebugContents, PpuContents, Window, WindowVisitor},
};
use nerust_gui_viewmodel::debugger::DisplayCache;
use nerust_settings_core::model::language::AppLanguage;
use rfd::FileDialog;

use crate::aux::bridge::DebugBridge;
#[cfg(test)]
use crate::aux::focus::AuxKind;
use crate::aux::registry::AuxController;
use crate::event::UserEvent;

/// Native file/error dialogs, seamed for headless tests. Production talks to rfd; fakes return
/// canned picks and record errors. `Send + Sync`: error dialogs run on the modal actor so the event loop never stalls behind a modal (see `ModalFlight`).
/// Titles arrive pre-localized from the host; ports never touch the i18n catalog.
pub(crate) trait DialogPorts: Send + Sync {
    fn pick_rom_file(&self, title: String, filters: Vec<(String, Vec<String>)>) -> Option<PathBuf>;
    fn show_load_error(&self, title: String, detail: &str);
    /// Non-blocking error dialog: spawns the modal, returns its ack channel.
    /// A dead channel means the spawn failed; the host reaps it as a failed ack.
    fn show_error_async(&self, title: String, detail: String) -> std::sync::mpsc::Receiver<()>;
}

/// Production dialogs over rfd.
pub(crate) struct NativeDialogs;

impl DialogPorts for NativeDialogs {
    fn pick_rom_file(&self, title: String, filters: Vec<(String, Vec<String>)>) -> Option<PathBuf> {
        let mut dialog = FileDialog::new().set_title(title);
        for (name, extensions) in filters {
            dialog = dialog.add_filter(name, extensions.as_slice());
        }
        dialog.pick_file()
    }

    fn show_load_error(&self, title: String, detail: &str) {
        show_rom_load_error(title, detail);
    }

    fn show_error_async(&self, title: String, detail: String) -> std::sync::mpsc::Receiver<()> {
        ModalActor::show(title, detail)
    }
}

/// Single modal actor: one queue, one worker thread for all error dialogs.
/// Per-batch spawn-a-thread is gone; a dead worker respawns on next use,
/// so one dialog-thread crash never disables later error modals.
struct ModalActor {
    tx: Option<std::sync::mpsc::Sender<(String, String, std::sync::mpsc::Sender<()>)>>,
}

impl ModalActor {
    /// Process-wide actor behind a lock; the worker (re)spawns on demand.
    fn global() -> &'static std::sync::Mutex<Self> {
        static ACTOR: std::sync::OnceLock<std::sync::Mutex<ModalActor>> =
            std::sync::OnceLock::new();
        ACTOR.get_or_init(|| std::sync::Mutex::new(ModalActor { tx: None }))
    }

    fn spawn_worker()
    -> Option<std::sync::mpsc::Sender<(String, String, std::sync::mpsc::Sender<()>)>> {
        let (tx, rx) = std::sync::mpsc::channel::<(String, String, std::sync::mpsc::Sender<()>)>();
        if let Err(error) = std::thread::Builder::new()
            .name("nerust-error-dialog".to_string())
            .spawn(move || {
                for (title, detail, done) in rx {
                    show_blocking(&title, &detail);
                    let _ = done.send(());
                }
            })
        {
            log::error!("error dialog worker failed to spawn: {error}");
            return None;
        }
        Some(tx)
    }

    /// Queue one modal: the worker shows it, then acks through the channel.
    /// A dead worker respawns once; only a spawn failure surfaces as a dead ack.
    fn show(title: String, detail: String) -> std::sync::mpsc::Receiver<()> {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let mut actor = Self::global()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if actor.tx.is_none() {
            actor.tx = Self::spawn_worker();
        }
        let mut sent = actor
            .tx
            .as_ref()
            .is_some_and(|tx| tx.send((title.clone(), detail.clone(), done_tx)).is_ok());
        if !sent {
            // Worker died between health check and send: one respawn, one retry.
            actor.tx = Self::spawn_worker();
            let (retry_tx, retry_rx) = std::sync::mpsc::channel();
            sent = actor
                .tx
                .as_ref()
                .is_some_and(|tx| tx.send((title, detail, retry_tx)).is_ok());
            if sent {
                return retry_rx;
            }
            log::error!("error dialog worker gone; ack will fail");
        }
        done_rx
    }
}

/// Blocking modal body: runs only on the actor worker thread.
fn show_blocking(title: &str, detail: &str) {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(detail)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Surface ROM load failures in a modal dialog: the previous
/// warn-only path left the empty window with no explanation.
pub(crate) fn show_rom_load_error(title: String, detail: &str) {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(detail)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Aux-window open parameters: one variant per capability.
pub(crate) enum AuxOpenParams {
    Controls {
        session: WidgetSession,
    },
    Debug {
        display: Box<DisplayCache>,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
        language: AppLanguage,
        position: Option<(i32, i32)>,
    },
    Ppu {
        bridge: Arc<DebugBridge>,
        position: Option<(i32, i32)>,
    },
}

/// Window creation surface: main plus one capability-routed aux entry.
/// Every method fails closed (`None`/`Err`) on headless fakes.
pub(crate) trait WindowOpener {
    fn create_main(&self, builder: WindowBuilder) -> Result<Arc<TaoWindow>, String>;
    /// The reserved common window selects the native program, not a second kind registry.
    fn create_aux(&self, window: Window, params: AuxOpenParams) -> Option<Box<dyn AuxController>>;
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

    fn create_aux(&self, window: Window, params: AuxOpenParams) -> Option<Box<dyn AuxController>> {
        let mut visitor = CreateAux {
            target: self.target,
            window: &window,
            params: Some(params),
            output: None,
        };
        window.visit(&mut visitor);
        visitor.output
    }
}

struct CreateAux<'a> {
    target: &'a EventLoopWindowTarget<UserEvent>,
    window: &'a Window,
    params: Option<AuxOpenParams>,
    output: Option<Box<dyn AuxController>>,
}

impl WindowVisitor for CreateAux<'_> {
    fn visit_controls(&mut self, _: &ControlScreen) {
        self.output = self
            .params
            .take()
            .and_then(controls_parts)
            .and_then(|session| {
                crate::aux::widgets::native::ControlsWindowHandle::new(
                    self.window.clone(),
                    session,
                    self.target,
                )
                .map(aux_box)
            });
    }
    fn visit_debug(&mut self, _: &DebugContents) {
        self.output = self.params.take().and_then(debug_parts).and_then(
            |(display, spaces, mem_addr, language, position)| {
                crate::aux::debug::DebugWindowHandle::new(
                    self.window.clone(),
                    display,
                    spaces,
                    mem_addr,
                    language,
                    self.target,
                    position,
                )
                .map(aux_box)
            },
        );
    }
    fn visit_ppu(&mut self, _: &PpuContents) {
        self.output = self
            .params
            .take()
            .and_then(ppu_parts)
            .and_then(|(bridge, position)| {
                crate::aux::ppu::PpuWindowHandle::new(
                    self.window.clone(),
                    bridge,
                    self.target,
                    position,
                )
                .map(aux_box)
            });
    }
}

/// Box a concrete aux handle behind its capability ports.
fn aux_box<T: AuxController + 'static>(handle: T) -> Box<dyn AuxController> {
    Box::new(handle)
}

/// Split one controls open: the variant check is pure, so tests cover
/// foreign-variant rejection without a display.
fn controls_parts(params: AuxOpenParams) -> Option<WidgetSession> {
    let AuxOpenParams::Controls { session } = params else {
        return None;
    };
    Some(session)
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
    Some((*display, spaces, mem_addr, language, position))
}

/// Split one viewer open: pure variant check, display-free tests.
type PpuParts = (Arc<DebugBridge>, Option<(i32, i32)>);

fn ppu_parts(params: AuxOpenParams) -> Option<PpuParts> {
    let AuxOpenParams::Ppu { bridge, position } = params else {
        return None;
    };
    Some((bridge, position))
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
            let mut host = crate::runtime::host::TestHost::new().host;
            let (_, session) =
                crate::aux::widgets::bindings::tests::prepare(&mut host, AuxKind::History);
            AuxOpenParams::Controls { session }
        }
        assert!(debug_parts(history_params()).is_none());
        assert!(ppu_parts(history_params()).is_none());
        assert!(controls_parts(history_params()).is_some());
        // Own variants split cleanly.
        let mut host = crate::runtime::host::TestHost::new().host;
        let (_, session) =
            crate::aux::widgets::bindings::tests::prepare(&mut host, AuxKind::Settings);
        assert!(controls_parts(AuxOpenParams::Controls { session }).is_some());
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
