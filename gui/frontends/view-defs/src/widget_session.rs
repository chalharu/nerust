//! Type-erased widget owner and opaque, cloneable native lifecycle handles.
//! No draft, domain event, or domain apply result crosses this boundary.

use std::{
    cell::RefCell,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::session::osd::{HistoryFilter, HistoryLine};
use nerust_settings_core::model::language::AppLanguage;

use wakui::{
    binding::{BindingTarget, PickerSink, ViewDefinition, drive},
    control::{ControlEffect, ControlEvent, ControlId, ControlScreen, Id},
    leaf::TextLeaf,
};

use wakui::mailbox::{MailboxDomain, WindowMailbox};
use wakui::sync::lock_recovered;
use wakui::window::Window;

use crate::{
    focus::AuxKind,
    history::{HistoryEffect, HistoryEvent, HistoryKey, HistoryModel},
    settings::{SettingsController, SettingsEffect, SettingsEvent},
};

use wakui::control::IdRegistry;

struct SettingsDomain {
    pending: Arc<Mutex<Option<SettingsSnapshot>>>,
}

impl MailboxDomain for SettingsDomain {
    fn has_pending(&self) -> bool {
        lock_recovered(&self.pending).is_some()
    }

    fn take_pending(&self) -> Option<Box<dyn std::any::Any>> {
        let snapshot = lock_recovered(&self.pending).take()?;
        Some(Box::new(Pending::Settings(Box::new(snapshot))))
    }
}

struct HistoryDomain {
    clear: Arc<AtomicBool>,
}

impl MailboxDomain for HistoryDomain {
    fn has_pending(&self) -> bool {
        self.clear.load(Ordering::Acquire)
    }

    fn take_pending(&self) -> Option<Box<dyn std::any::Any>> {
        if !self.clear.swap(false, Ordering::AcqRel) {
            return None;
        }
        Some(Box::new(Pending::ClearHistory))
    }
}

/// Consume a pending batch from a retained handle. Native adapters call this
/// when the matching generation can receive it; the payload stays view-side.
pub fn take_submission(mailbox: &WindowMailbox) -> Option<WindowSubmission> {
    let pending = mailbox
        .take_pending_any()?
        .downcast::<Pending>()
        .ok()
        .map(|boxed| *boxed)?;
    Some(WindowSubmission {
        mailbox: mailbox.clone(),
        pending: Arc::new(Mutex::new(Some(pending))),
    })
}

pub enum Pending {
    Settings(Box<SettingsSnapshot>),
    ClearHistory,
}

/// Clones share a single consumable batch, not a pointer to future submissions.
#[derive(Clone)]
pub struct WindowSubmission {
    pub mailbox: WindowMailbox,
    pub pending: Arc<Mutex<Option<Pending>>>,
}

impl WindowSubmission {
    pub fn take(&self) -> Option<Pending> {
        lock_recovered(&self.pending).take()
    }
}

enum State {
    Settings(Box<SettingsController>),
    History(HistorySession),
    Ppu(PpuState),
    Debug(Box<DebugWidget>),
}

struct PpuState {
    session: RefCell<crate::ppu::PpuSession>,
    language: AppLanguage,
}

struct DebugWidget {
    handle: crate::debugger::DebugHandle,
    session: RefCell<crate::debugger::DebuggerScreenSession>,
}

/// Bound history instance. Owns filter state plus the stable ID registry;
/// the view itself stays a fresh snapshot pair each frame.
pub(crate) struct HistorySession {
    lines: Arc<[HistoryLine]>,
    language: AppLanguage,
    filter: HistoryFilter,
    ids: RefCell<IdRegistry<HistoryKey>>,
    mailbox: WindowMailbox,
    clear: Arc<AtomicBool>,
}

impl wakui::binding::BindingTarget for HistorySession {
    type DomainEvent = HistoryEvent;
    type Effect = HistoryEffect;

    fn apply(&mut self, event: HistoryEvent) -> Option<HistoryEffect> {
        match event {
            HistoryEvent::SetFilter(filter) => {
                self.filter = filter;
            }
            HistoryEvent::Clear => {
                self.clear.store(true, Ordering::Release);
            }
            HistoryEvent::Close => {
                self.mailbox.request_close();
            }
        }
        self.mailbox.invalidate();
        None
    }
}

/// Native code owns one of these, without knowing which domain controller it wraps.
pub struct WidgetSession {
    state: State,
    mailbox: WindowMailbox,
    picker: Option<(ControlId, Id)>,
}

impl WidgetSession {
    /// Keep the typed draft private behind the common native boundary.
    pub fn from_settings(mut controller: SettingsController) -> Self {
        let mailbox = WindowMailbox::new(SettingsDomain {
            pending: controller.mailbox.pending_apply.clone(),
        });
        controller.mailbox.should_close = mailbox.flags().close.clone();
        controller.mailbox.view_invalidated = mailbox.flags().invalidated.clone();
        Self {
            state: State::Settings(Box::new(controller)),
            mailbox,
            picker: None,
        }
    }

    pub fn history(lines: Arc<[HistoryLine]>, language: AppLanguage) -> Self {
        let clear = Arc::new(AtomicBool::new(false));
        let mailbox = WindowMailbox::new(HistoryDomain {
            clear: clear.clone(),
        });
        Self {
            state: State::History(HistorySession {
                lines,
                language,
                filter: HistoryFilter::All,
                ids: RefCell::new(IdRegistry::default()),
                mailbox: mailbox.clone(),
                clear,
            }),
            mailbox,
            picker: None,
        }
    }

    pub fn ppu(
        chrome: crate::ppu::PpuChrome,
        pictures: Vec<crate::ppu::PpuPicture>,
        panels: Option<String>,
        generation: u64,
        language: AppLanguage,
    ) -> Self {
        let mailbox = WindowMailbox::new(HistoryDomain {
            // Reuse the flag-only domain: PPU has no pending submission,
            // only invalidation on refresh/hover.
            clear: Arc::new(AtomicBool::new(false)),
        });
        Self {
            state: State::Ppu(PpuState {
                session: RefCell::new(crate::ppu::PpuSession::new(
                    chrome, pictures, panels, generation,
                )),
                language,
            }),
            mailbox,
            picker: None,
        }
    }

    /// Debugger widget: projects the cached bridge snapshot as controls.
    /// Snapshots arrive via [`Self::refresh_debug`]; gestures map via
    /// [`Self::debug_events`] and run through the host's debug driver.
    pub fn debug(handle: crate::debugger::DebugHandle) -> Self {
        let mailbox = WindowMailbox::new(HistoryDomain {
            // Flag-only domain like PPU: no submission, only invalidation.
            clear: Arc::new(AtomicBool::new(false)),
        });
        Self {
            state: State::Debug(Box::new(DebugWidget {
                handle,
                session: RefCell::new(crate::debugger::DebuggerScreenSession::new()),
            })),
            mailbox,
            picker: None,
        }
    }

    pub fn debug_handle(&self) -> Option<crate::debugger::DebugHandle> {
        match &self.state {
            State::Debug(widget) => Some(widget.handle),
            _ => None,
        }
    }

    /// Replace the cached debugger snapshot and invalidate when it changed
    /// presentation. Always invalidates on first snapshot.
    pub fn refresh_debug(&mut self, snapshot: Option<crate::debugger::DebugSnapshot>) {
        if let State::Debug(widget) = &mut self.state {
            widget.session.borrow_mut().update(snapshot);
            self.mailbox.invalidate();
        }
    }

    /// Map a generic control event to debugger gestures. `None` for non-debug
    /// sessions; the host runs the gestures through its debug driver.
    pub fn debug_events(
        &self,
        event: &ControlEvent,
    ) -> Option<Vec<crate::debugger::DebuggerEvent>> {
        match &self.state {
            State::Debug(widget) => Some(widget.session.borrow().events(event)),
            _ => None,
        }
    }

    pub fn mailbox(&self) -> WindowMailbox {
        self.mailbox.clone()
    }

    pub fn screen(&self) -> ControlScreen {
        let mut screen = match &self.state {
            State::Settings(controller) => controller.screen().screen,
            State::Ppu(state) => state.session.borrow_mut().screen(),
            State::Debug(widget) => widget.session.borrow_mut().screen(),
            State::History(session) => {
                let model = HistoryModel::build(&session.lines, session.language, session.filter);
                model.screen(&mut session.ids.borrow_mut()).0
            }
        };
        if let Some(error) = lock_recovered(&self.mailbox.flags().error).clone() {
            screen.message = Some(error);
        }
        screen
    }

    pub fn window(&self, id: wakui::aux::AuxId) -> Window {
        Window::controls(id, self.screen())
    }

    pub fn kind(&self) -> AuxKind {
        match self.state {
            State::Settings(_) => AuxKind::Settings,
            State::History(_) => AuxKind::History,
            State::Ppu(_) => AuxKind::Ppu,
            State::Debug(_) => AuxKind::Debug,
        }
    }

    pub fn refresh_ppu(
        &mut self,
        chrome: crate::ppu::PpuChrome,
        pictures: Vec<crate::ppu::PpuPicture>,
        panels: Option<String>,
        generation: u64,
        language: AppLanguage,
    ) {
        if let State::Ppu(state) = &mut self.state {
            state.language = language;
            if state
                .session
                .borrow_mut()
                .refresh(chrome, pictures, panels, generation)
            {
                self.mailbox.invalidate();
            }
        }
    }

    pub fn refresh_history(&mut self, lines: Arc<[HistoryLine]>, language: AppLanguage) {
        if let State::History(session) = &mut self.state
            && (session.lines != lines || session.language != language)
        {
            session.lines = lines;
            session.language = language;
            session.mailbox.invalidate();
        }
    }

    /// Resolve callbacks against current topology and metadata, not retained AST
    /// bindings. The returned effects own all data; execute after releasing borrows.
    /// Picker results never arrive here; see [`PickerSink`] instead.
    pub fn dispatch(&mut self, event: ControlEvent) -> Vec<ControlEffect> {
        let screen = self.screen();
        let Some(id) = event.id.or(screen.capture.target) else {
            return Vec::new();
        };
        if !screen.control(id).is_some_and(|c| c.accepts(&event.input)) {
            return Vec::new();
        }
        let mailbox = self.mailbox.clone();
        if lock_recovered(&mailbox.flags().error).take().is_some() {
            mailbox.invalidate();
        }
        match &mut self.state {
            State::Settings(controller) => {
                let view = controller.screen();
                let mut effects = Vec::new();
                for effect in drive(&view, &id, event.input, &mut **controller) {
                    let SettingsEffect::BrowseStorageDirectory { title } = effect;
                    let request = Id::fresh();
                    self.picker = Some((id, request));
                    effects.push(ControlEffect::PickDirectory {
                        title,
                        target: id,
                        request,
                    });
                }
                effects
            }
            State::History(session) => {
                let model = HistoryModel::build(&session.lines, session.language, session.filter);
                let (_, ids) = model.screen(&mut session.ids.borrow_mut());
                let action = if id == ids.filter {
                    Some(crate::history::HistoryViewId::Filter)
                } else if id == ids.clear {
                    Some(crate::history::HistoryViewId::Clear)
                } else if id == ids.close {
                    Some(crate::history::HistoryViewId::Close)
                } else {
                    None
                };
                let Some(action) = action else {
                    return Vec::new();
                };
                let Some(input) = HistoryModel::convert(event.input) else {
                    return Vec::new();
                };
                let _ = drive(&model, &action, input, session);
                Vec::new()
            }
            State::Ppu(state) => {
                // Hover is the only PPU input; formatting lives in the
                // view definition, natives only route positions.
                let language = state.language;
                let changed = state
                    .session
                    .borrow_mut()
                    .dispatch(&screen, &event, language);
                if changed {
                    mailbox.invalidate();
                }
                Vec::new()
            }
            State::Debug(_) => {
                // Debugger gestures run host-side through the debug driver
                // (needs the bridge handle); see `debug_events`. Reaching
                // here means the host bypassed that path: ignore.
                Vec::new()
            }
        }
    }

    /// Mint history action IDs without projecting rows. Used by tests.
    #[cfg(test)]
    pub(crate) fn history_ids(&mut self) -> crate::history::HistoryIds {
        let State::History(session) = &mut self.state else {
            panic!("history session required");
        };
        let model = HistoryModel::build(&session.lines, session.language, session.filter);
        model.screen(&mut session.ids.borrow_mut()).1
    }
}

impl PickerSink<std::path::PathBuf> for WidgetSession {
    /// Deliver a native folder-picker result. Validates the pending request
    /// and the target's current browse affordance, mirroring dispatch's gates.
    fn submit(&mut self, target: ControlId, request: Id, payload: Option<std::path::PathBuf>) {
        if self.picker != Some((target, request)) {
            return;
        }
        self.picker = None;
        let mailbox = self.mailbox.clone();
        if lock_recovered(&mailbox.flags().error).take().is_some() {
            mailbox.invalidate();
        }
        let screen = self.screen();
        let State::Settings(controller) = &mut self.state else {
            return;
        };
        let browseable = screen.control(target).is_some_and(|c| {
            c.downcast::<TextLeaf>()
                .is_some_and(|leaf| leaf.browse_label.is_some())
        });
        if !browseable {
            return;
        }
        let _ = controller.apply(SettingsEvent::StorageDirectorySelected(payload));
    }
}

#[cfg(test)]
mod tests;
