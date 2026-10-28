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

use crate::{
    binding::{BindingTarget as _, ViewDefinition as _},
    control::{ControlEffect, ControlEvent, ControlId, ControlKind, ControlScreen},
    debugger::lock_recovered,
    focus::AuxKind,
    history::{HistoryEffect, HistoryEvent, HistoryModel},
    settings::{
        SettingsController, SettingsEffect, SettingsEvent, SettingsMailbox, SettingsScreen,
    },
    window::Window,
};

mod scene;

enum MailboxSource {
    Settings(SettingsMailbox),
    History {
        close: AtomicBool,
        invalidated: AtomicBool,
        clear: AtomicBool,
    },
}

struct MailboxInner {
    source: MailboxSource,
    error: Mutex<Option<String>>,
}

/// Native registries retain this handle, never SettingsSnapshot or VM state.
#[derive(Clone)]
pub struct WindowMailbox(Arc<MailboxInner>);

pub(crate) enum Pending {
    Settings(Box<SettingsSnapshot>),
    ClearHistory,
}

/// Clones share a single consumable batch, not a pointer to future submissions.
#[derive(Clone)]
pub struct WindowSubmission {
    pub(crate) mailbox: WindowMailbox,
    pending: Arc<Mutex<Option<Pending>>>,
}

impl WindowSubmission {
    pub(crate) fn take(&self) -> Option<Pending> {
        lock_recovered(&self.pending).take()
    }
}

impl WindowMailbox {
    fn new(source: MailboxSource) -> Self {
        Self(Arc::new(MailboxInner {
            source,
            error: Mutex::new(None),
        }))
    }

    pub fn close_requested(&self) -> bool {
        match &self.0.source {
            MailboxSource::Settings(m) => m.should_close.load(Ordering::Acquire),
            MailboxSource::History { close, .. } => close.load(Ordering::Acquire),
        }
    }

    pub fn request_close(&self) {
        self.set_close(true);
    }

    fn set_close(&self, value: bool) {
        match &self.0.source {
            MailboxSource::Settings(m) => m.should_close.store(value, Ordering::Release),
            MailboxSource::History { close, .. } => close.store(value, Ordering::Release),
        }
    }

    pub fn invalidate(&self) {
        match &self.0.source {
            MailboxSource::Settings(m) => m.view_invalidated.store(true, Ordering::Release),
            MailboxSource::History { invalidated, .. } => {
                invalidated.store(true, Ordering::Release)
            }
        }
    }

    pub fn take_invalidated(&self) -> bool {
        match &self.0.source {
            MailboxSource::Settings(m) => m.view_invalidated.swap(false, Ordering::AcqRel),
            MailboxSource::History { invalidated, .. } => invalidated.swap(false, Ordering::AcqRel),
        }
    }

    pub fn has_pending(&self) -> bool {
        match &self.0.source {
            MailboxSource::Settings(m) => lock_recovered(&m.pending_apply).is_some(),
            MailboxSource::History { clear, .. } => clear.load(Ordering::Acquire),
        }
    }

    pub fn take_submission(&self) -> Option<WindowSubmission> {
        let pending = match &self.0.source {
            MailboxSource::Settings(m) => Pending::Settings(Box::new(m.take_pending_apply()?)),
            MailboxSource::History { clear, .. } => {
                if !clear.swap(false, Ordering::AcqRel) {
                    return None;
                }
                Pending::ClearHistory
            }
        };
        Some(WindowSubmission {
            mailbox: self.clone(),
            pending: Arc::new(Mutex::new(Some(pending))),
        })
    }

    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn apply_failed(&self, error: String) {
        *lock_recovered(&self.0.error) = Some(error);
        self.set_close(false);
        self.invalidate();
    }
}

impl crate::binding::Invalidatable for WindowMailbox {
    fn invalidate(&self) {
        WindowMailbox::invalidate(self);
    }

    fn take_invalidated(&self) -> bool {
        WindowMailbox::take_invalidated(self)
    }
}

enum State {
    Settings(Box<SettingsController>),
    History(HistorySession),
}

/// Bound history instance. Owns filter state and mailbox effects;
/// the view itself stays a fresh `HistoryModel` snapshot.
pub(crate) struct HistorySession {
    lines: Arc<[HistoryLine]>,
    language: AppLanguage,
    filter: HistoryFilter,
    mailbox: WindowMailbox,
}

impl crate::binding::BindingTarget for HistorySession {
    type DomainEvent = HistoryEvent;
    type Effect = HistoryEffect;

    fn apply(&mut self, event: HistoryEvent) -> Option<HistoryEffect> {
        match event {
            HistoryEvent::SetFilter(filter) => {
                self.filter = filter;
            }
            HistoryEvent::Clear => {
                if let MailboxSource::History { clear, .. } = &self.mailbox.0.source {
                    clear.store(true, Ordering::Release);
                }
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
    scene: RefCell<scene::SceneBuilder>,
    mailbox: WindowMailbox,
    picker: Option<(ControlId, crate::control::Id)>,
}

impl WidgetSession {
    /// Keep the typed draft private behind the common native boundary.
    pub(crate) fn from_settings(controller: SettingsController) -> Self {
        let mailbox = WindowMailbox::new(MailboxSource::Settings(controller.mailbox.clone()));
        Self {
            state: State::Settings(Box::new(controller)),
            scene: RefCell::new(Default::default()),
            mailbox,
            picker: None,
        }
    }

    pub(crate) fn history(lines: Arc<[HistoryLine]>, language: AppLanguage) -> Self {
        let mailbox = WindowMailbox::new(MailboxSource::History {
            close: AtomicBool::new(false),
            invalidated: AtomicBool::new(false),
            clear: AtomicBool::new(false),
        });
        Self {
            state: State::History(HistorySession {
                lines,
                language,
                filter: HistoryFilter::All,
                mailbox: mailbox.clone(),
            }),
            scene: RefCell::new(Default::default()),
            mailbox,
            picker: None,
        }
    }

    pub fn mailbox(&self) -> WindowMailbox {
        self.mailbox.clone()
    }

    pub fn screen(&self) -> ControlScreen {
        let mut scene = self.scene.borrow_mut();
        let mut screen = match &self.state {
            State::Settings(controller) => scene.settings(&controller.screen()).0,
            State::History(session) => scene.history(&HistoryModel::build(
                &session.lines,
                session.language,
                session.filter,
            )),
        };
        if let Some(error) = lock_recovered(&self.mailbox.0.error).clone() {
            screen.message = Some(error);
        }
        screen
    }

    pub fn window(&self) -> Window {
        Window::controls(self.kind(), self.screen())
    }

    pub(crate) fn kind(&self) -> AuxKind {
        match self.state {
            State::Settings(_) => AuxKind::Settings,
            State::History(_) => AuxKind::History,
        }
    }

    pub(crate) fn refresh_history(&mut self, lines: Arc<[HistoryLine]>, language: AppLanguage) {
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
        if lock_recovered(&mailbox.0.error).take().is_some() {
            mailbox.invalidate();
        }
        match &mut self.state {
            State::Settings(controller) => {
                let typed = controller.screen();
                let (_, bindings) = self.scene.borrow_mut().settings(&typed);
                let Some((_, domain_id)) = bindings.iter().find(|(token, _)| *token == id) else {
                    return Vec::new();
                };
                let events = SettingsScreen::convert(event.input)
                    .and_then(|input| typed.bind(domain_id, input));
                let mut effects = Vec::new();
                for event in events.into_iter().flatten() {
                    if let Some(SettingsEffect::BrowseStorageDirectory { title }) =
                        controller.apply(event)
                    {
                        let request = crate::control::Id::fresh();
                        self.picker = Some((id, request));
                        effects.push(ControlEffect::PickDirectory {
                            title,
                            target: id,
                            request,
                        });
                    }
                }
                effects
            }
            State::History(session) => {
                let action = match self.scene.borrow_mut().history_action(id) {
                    Some(action) => action,
                    None => return Vec::new(),
                };
                let input = match HistoryModel::convert(event.input) {
                    Some(input) => input,
                    None => return Vec::new(),
                };
                let model = HistoryModel::build(&session.lines, session.language, session.filter);
                let events = match model.bind(&action, input) {
                    Some(events) => events,
                    None => return Vec::new(),
                };
                for event in events {
                    let _ = session.apply(event);
                }
                Vec::new()
            }
        }
    }
}

impl crate::binding::PickerSink<std::path::PathBuf> for WidgetSession {
    /// Deliver a native folder-picker result. Validates the pending request
    /// and the target's current browse affordance, mirroring dispatch's gates.
    fn submit(
        &mut self,
        target: ControlId,
        request: crate::control::Id,
        payload: Option<std::path::PathBuf>,
    ) {
        if self.picker != Some((target, request)) {
            return;
        }
        self.picker = None;
        let mailbox = self.mailbox.clone();
        if lock_recovered(&mailbox.0.error).take().is_some() {
            mailbox.invalidate();
        }
        let screen = self.screen();
        let State::Settings(controller) = &mut self.state else {
            return;
        };
        let browseable = screen.control(target).is_some_and(|c| {
            matches!(
                &c.kind,
                ControlKind::Text {
                    browse_label: Some(_),
                    ..
                }
            )
        });
        if !browseable {
            return;
        }
        let _ = controller.apply(SettingsEvent::StorageDirectorySelected(payload));
    }
}

#[cfg(test)]
mod tests;
