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
    binding::{BindingTarget, ViewDefinition, drive},
    control::{ControlEffect, ControlEvent, ControlId, ControlKind, ControlScreen},
};

use wakui::mailbox::{MailboxDomain, WindowMailbox};

use crate::{
    focus::AuxKind,
    history::{HistoryEffect, HistoryEvent, HistoryModel},
    settings::{SettingsController, SettingsEffect, SettingsEvent, SettingsScreen},
    sync::lock_recovered,
    window::Window,
};

mod scene;

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
    pending: Arc<Mutex<Option<Pending>>>,
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
}

struct PpuState {
    session: RefCell<crate::ppu::PpuSession>,
    language: AppLanguage,
}

/// Bound history instance. Owns filter state and mailbox effects;
/// the view itself stays a fresh `HistoryModel` snapshot.
pub(crate) struct HistorySession {
    lines: Arc<[HistoryLine]>,
    language: AppLanguage,
    filter: HistoryFilter,
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
    scene: RefCell<scene::SceneBuilder>,
    mailbox: WindowMailbox,
    picker: Option<(ControlId, wakui::control::Id)>,
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
            scene: RefCell::new(Default::default()),
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
                mailbox: mailbox.clone(),
                clear,
            }),
            scene: RefCell::new(Default::default()),
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
            State::Ppu(state) => state.session.borrow_mut().screen(),
            State::History(session) => scene.history(&HistoryModel::build(
                &session.lines,
                session.language,
                session.filter,
            )),
        };
        if let Some(error) = lock_recovered(&self.mailbox.flags().error).clone() {
            screen.message = Some(error);
        }
        screen
    }

    pub fn window(&self) -> Window {
        Window::controls(self.kind(), self.screen())
    }

    pub fn kind(&self) -> AuxKind {
        match self.state {
            State::Settings(_) => AuxKind::Settings,
            State::History(_) => AuxKind::History,
            State::Ppu(_) => AuxKind::Ppu,
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
                let typed = controller.screen();
                let (_, bindings) = self.scene.borrow_mut().settings(&typed);
                let Some((_, domain_id)) = bindings.iter().find(|(token, _)| *token == id) else {
                    return Vec::new();
                };
                let Some(input) = SettingsScreen::convert(event.input) else {
                    return Vec::new();
                };
                let mut effects = Vec::new();
                for effect in drive(&typed, domain_id, input, &mut **controller) {
                    let SettingsEffect::BrowseStorageDirectory { title } = effect;
                    let request = wakui::control::Id::fresh();
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
                let action = match self.scene.borrow_mut().history_action(id) {
                    Some(action) => action,
                    None => return Vec::new(),
                };
                let Some(input) = HistoryModel::convert(event.input) else {
                    return Vec::new();
                };
                let model = HistoryModel::build(&session.lines, session.language, session.filter);
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
        }
    }
}

impl wakui::binding::PickerSink<std::path::PathBuf> for WidgetSession {
    /// Deliver a native folder-picker result. Validates the pending request
    /// and the target's current browse affordance, mirroring dispatch's gates.
    fn submit(
        &mut self,
        target: ControlId,
        request: wakui::control::Id,
        payload: Option<std::path::PathBuf>,
    ) {
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
