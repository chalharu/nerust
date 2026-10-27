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
    control::{ControlEffect, ControlEvent, ControlId, ControlInput, ControlScreen},
    debugger::lock_recovered,
    focus::AuxKind,
    history::HistoryModel,
    settings::{SettingsController, SettingsEffect, SettingsEvent, SettingsMailbox},
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

enum State {
    Settings(Box<SettingsController>),
    History {
        lines: Arc<[HistoryLine]>,
        language: AppLanguage,
        filter: HistoryFilter,
    },
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
        Self {
            state: State::History {
                lines,
                language,
                filter: HistoryFilter::All,
            },
            scene: RefCell::new(Default::default()),
            mailbox: WindowMailbox::new(MailboxSource::History {
                close: AtomicBool::new(false),
                invalidated: AtomicBool::new(false),
                clear: AtomicBool::new(false),
            }),
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
            State::History {
                lines,
                language,
                filter,
            } => scene.history(&HistoryModel::build(lines, *language, *filter)),
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
            State::History { .. } => AuxKind::History,
        }
    }

    pub(crate) fn refresh_history(&mut self, lines: Arc<[HistoryLine]>, language: AppLanguage) {
        if let State::History {
            lines: current,
            language: current_language,
            ..
        } = &mut self.state
            && (*current != lines || *current_language != language)
        {
            *current = lines;
            *current_language = language;
            self.mailbox.invalidate();
        }
    }

    /// Resolve callbacks against current topology and metadata, not retained AST
    /// bindings. The returned effects own all data; execute after releasing borrows.
    pub fn dispatch(&mut self, event: ControlEvent) -> Vec<ControlEffect> {
        let screen = self.screen();
        let Some(id) = event.id.or(screen.capture.target) else {
            return Vec::new();
        };
        if let ControlInput::DirectorySelected { request, .. } = &event.input {
            if self.picker != Some((id, *request)) {
                return Vec::new();
            }
            self.picker = None;
        }
        if !screen.control(id).is_some_and(|c| c.accepts(&event.input)) {
            return Vec::new();
        }
        if lock_recovered(&self.mailbox.0.error).take().is_some() {
            self.mailbox.invalidate();
        }
        match &mut self.state {
            State::Settings(controller) => {
                let typed = controller.screen();
                let (_, bindings) = self.scene.borrow_mut().settings(&typed);
                let Some((_, domain_id)) = bindings.iter().find(|(token, _)| *token == id) else {
                    return Vec::new();
                };
                let events = if let ControlInput::DirectorySelected { path, .. } = event.input {
                    Some(vec![SettingsEvent::StorageDirectorySelected(path)])
                } else {
                    scene::settings_input(event.input)
                        .and_then(|input| typed.bind(domain_id, input))
                };
                let mut effects = Vec::new();
                for event in events.into_iter().flatten() {
                    if let Some(SettingsEffect::BrowseStorageDirectory { title }) =
                        controller.update(event)
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
            State::History { filter, .. } => {
                let mut scene = self.scene.borrow_mut();
                if id == scene.history_filter() {
                    if let ControlInput::Select(index) = event.input {
                        *filter = HistoryFilter::all()[index];
                    }
                } else if id == scene.history_clear() {
                    if let MailboxSource::History { clear, .. } = &self.mailbox.0.source {
                        clear.store(true, Ordering::Release);
                    }
                } else if id == scene.history_close() {
                    self.mailbox.request_close();
                }
                self.mailbox.invalidate();
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests;
