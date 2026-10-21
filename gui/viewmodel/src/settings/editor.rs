use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

use nerust_settings_core::model::snapshot::SettingsSnapshot;

use super::{
    ValidationState,
    catalog::FactoryCatalog,
    projection::ProjectionHub,
    property::{ObservablePropertyInner, ReadOnlyObservableProperty},
};

#[cfg(test)]
pub use super::state::NoopStoragePathValidator;
pub use super::state::{EditorState, StoragePathValidator, ViewModelError};

/// Lightweight handle to the shared editor state and projection hub.
#[derive(Clone)]
pub struct SettingsEditor {
    current: Rc<RefCell<EditorState>>,
    projections: ProjectionHub,
    validator: Rc<dyn Fn(&EditorState) -> ValidationState>,
    notifying: Rc<Cell<bool>>,
    revision_inner: Rc<ObservablePropertyInner<u64>>,
}

impl SettingsEditor {
    #[allow(private_interfaces)]
    pub fn new(
        snapshot: SettingsSnapshot,
        catalog: FactoryCatalog,
        supported_sample_rates: Arc<[u32]>,
        storage_validator: Rc<dyn StoragePathValidator>,
        validator: impl Fn(&EditorState) -> ValidationState + 'static,
    ) -> Self {
        let draft = Arc::new(snapshot);
        let editor_state = EditorState {
            draft,
            capture_target: None,
            validation: ValidationState { issues: vec![] },
            revision: 0,
            catalog,
            supported_sample_rates,
            storage_validator,
            conflicts_cache: RefCell::new(None),
            topology_cache: RefCell::new(Default::default()),
        };
        let initial_validation = validator(&editor_state);
        let current = Rc::new(RefCell::new(EditorState {
            validation: initial_validation,
            ..editor_state
        }));

        let revision_inner = Rc::new(ObservablePropertyInner::new(0u64));

        Self {
            current,
            projections: ProjectionHub::new(),
            validator: Rc::new(validator),
            notifying: Rc::new(Cell::new(false)),
            revision_inner,
        }
    }

    pub fn revision_prop(&self) -> ReadOnlyObservableProperty<u64> {
        ReadOnlyObservableProperty::new(Rc::clone(&self.revision_inner))
    }

    pub fn current(&self) -> std::cell::Ref<'_, EditorState> {
        self.current.borrow()
    }

    pub fn revision(&self) -> u64 {
        self.current.borrow().revision
    }

    pub fn snapshot(&self) -> SettingsSnapshot {
        self.current.borrow().draft.as_ref().clone()
    }

    pub(crate) fn projections(&self) -> &ProjectionHub {
        &self.projections
    }

    pub fn transact<R>(
        &self,
        mutate: impl FnOnce(&mut EditorState) -> Result<R, ViewModelError>,
    ) -> Result<R, ViewModelError> {
        if self.notifying.get() {
            return Err(ViewModelError::ReentrantMutation);
        }

        let mut candidate = self.current.borrow().clone();
        let result = mutate(&mut candidate)?;
        let previous = self.current.borrow();
        let draft_changed =
            !Arc::ptr_eq(&candidate.draft, &previous.draft) && *candidate.draft != *previous.draft;
        if !draft_changed && candidate.capture_target == previous.capture_target {
            return Ok(result);
        }
        if draft_changed {
            candidate.validation = (self.validator)(&candidate);
        } else {
            candidate.draft = Arc::clone(&previous.draft);
            candidate.conflicts_cache = previous.conflicts_cache.clone();
            candidate.topology_cache = previous.topology_cache.clone();
        }
        candidate.revision = previous.revision.wrapping_add(1);
        drop(previous);
        let rev_value = candidate.revision;
        let prepared = self.projections.prepare_all(&candidate);

        *self.current.borrow_mut() = candidate;

        let mut notifications: Vec<Box<dyn FnOnce()>> = Vec::new();
        for projection in prepared {
            if let Some(notify) = projection.apply() {
                notifications.push(notify);
            }
        }

        let rev_callbacks = self.revision_inner.set(rev_value);

        let _guard = NotificationGuard::enter(&self.notifying);
        for notify in notifications {
            notify();
        }
        if let Some(callbacks) = rev_callbacks {
            for cb in &callbacks {
                cb(&rev_value);
            }
        }

        Ok(result)
    }

    pub fn finish(&self) -> Result<SettingsSnapshot, ValidationState> {
        let state = self.current.borrow();
        if state.validation.can_submit() {
            Ok((*state.draft).clone())
        } else {
            Err(state.validation.clone())
        }
    }
}

struct NotificationGuard(Rc<Cell<bool>>);

impl NotificationGuard {
    fn enter(notifying: &Rc<Cell<bool>>) -> Self {
        notifying.set(true);
        Self(Rc::clone(notifying))
    }
}

impl Drop for NotificationGuard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_settings_core::model::{
        app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
    };

    fn empty_snapshot() -> SettingsSnapshot {
        SettingsSnapshot {
            shared: DesktopSharedSettings::default(),
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        }
    }

    fn test_editor() -> SettingsEditor {
        let catalog = crate::settings::catalog::FactoryCatalog::new(Vec::new()).unwrap();
        let noop = Rc::new(NoopStoragePathValidator);
        let always_valid = |_: &EditorState| ValidationState { issues: vec![] };
        SettingsEditor::new(empty_snapshot(), catalog, Arc::new([]), noop, always_valid)
    }

    #[test]
    fn transact_noop_does_not_advance_revision() {
        let editor = test_editor();
        let rev_before = editor.revision();
        let result: Result<(), ViewModelError> = editor.transact(|_| Ok(()));
        assert!(result.is_ok());
        assert_eq!(editor.revision(), rev_before);
    }

    #[test]
    fn capture_only_transaction_reuses_draft_and_validation() {
        let validations = Rc::new(Cell::new(0));
        let calls = Rc::clone(&validations);
        let editor = SettingsEditor::new(
            empty_snapshot(),
            FactoryCatalog::new(vec![]).unwrap(),
            Arc::new([]),
            Rc::new(NoopStoragePathValidator),
            move |_| {
                calls.set(calls.get() + 1);
                ValidationState { issues: vec![] }
            },
        );
        let draft = Arc::clone(&editor.current().draft);
        editor
            .transact(|state| {
                state.capture_target = Some(nerust_settings_core::editor::CaptureTarget::Shortcut(
                    nerust_settings_core::model::input::ShortcutAction::TogglePause,
                ));
                Ok(())
            })
            .unwrap();
        assert_eq!(validations.get(), 1, "capture does not revalidate settings");
        assert!(Arc::ptr_eq(&draft, &editor.current().draft));
        assert_eq!(editor.revision(), 1);
        editor
            .transact(|state| {
                state.draft_mut().local.audio.muted = true;
                Ok(())
            })
            .unwrap();
        assert_eq!(validations.get(), 2);
        assert!(!Arc::ptr_eq(&draft, &editor.current().draft));
    }

    #[test]
    fn transact_mutation_advances_revision() {
        let editor = test_editor();
        let result: Result<(), ViewModelError> = editor.transact(|state| {
            state.draft_mut().local.audio.muted = true;
            Ok(())
        });
        assert!(result.is_ok());
        assert_eq!(editor.revision(), 1);
    }

    #[test]
    fn transact_error_does_not_change_state() {
        let editor = test_editor();
        let result: Result<(), ViewModelError> = editor.transact(|state| {
            state.draft_mut().local.audio.muted = true;
            Err(ViewModelError::UnknownSystem("test".into()))
        });
        assert!(result.is_err());
        assert!(!editor.current().draft.local.audio.muted);
    }

    #[test]
    fn finish_returns_draft_on_valid() {
        let editor = test_editor();
        assert!(editor.finish().is_ok());
    }

    #[test]
    fn finish_rejects_invalid_state() {
        let editor = test_editor();
        editor.current.borrow_mut().validation = ValidationState {
            issues: vec![super::super::ValidationIssue {
                scope: super::super::ValidationScope::Persistence,
                message: "test error".into(),
            }],
        };
        assert!(editor.finish().is_err());
    }

    #[test]
    fn noop_mutation_skips_validation() {
        let editor = test_editor();
        editor.current.borrow_mut().validation = ValidationState {
            issues: vec![super::super::ValidationIssue {
                scope: super::super::ValidationScope::Persistence,
                message: "test".into(),
            }],
        };
        let result: Result<(), ViewModelError> = editor.transact(|_| Ok(()));
        assert!(result.is_ok());
        assert!(!editor.current().validation.can_submit());
    }

    #[test]
    fn validator_checks_storage_directory() {
        let catalog = crate::settings::catalog::FactoryCatalog::new(Vec::new()).unwrap();
        let noop = Rc::new(NoopStoragePathValidator);
        let check_storage = |state: &EditorState| {
            if state.draft.shared.persistence.storage_directory.is_none() {
                ValidationState {
                    issues: vec![super::super::ValidationIssue {
                        scope: super::super::ValidationScope::Persistence,
                        message: "no storage dir".into(),
                    }],
                }
            } else {
                ValidationState { issues: vec![] }
            }
        };
        let editor =
            SettingsEditor::new(empty_snapshot(), catalog, Arc::new([]), noop, check_storage);
        assert!(!editor.current().validation.can_submit());
    }

    #[test]
    fn projection_observer_fires_after_transaction() {
        use std::rc::Rc;

        let editor = test_editor();

        let prop: ReadOnlyObservableProperty<bool> =
            editor
                .projections()
                .register("test_proj", false, |state| state.draft.local.audio.muted);

        let observed = Rc::new(Cell::new(false));
        let observed_cb = Rc::clone(&observed);
        let _sub = prop.observe(move |v| {
            observed_cb.set(*v);
        });

        editor
            .transact(|state| {
                state.draft_mut().local.audio.muted = true;
                Ok(())
            })
            .unwrap();

        assert!(
            observed.get(),
            "projection observer should have fired after transact"
        );
    }
}
