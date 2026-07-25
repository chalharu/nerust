#![allow(dead_code)]

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    sync::Arc,
};

use nerust_gui_settings::snapshot::SettingsSnapshot;
use nerust_settings_core::editor::CaptureTarget;

use super::catalog::FactoryCatalog;

/// Storage path validation port.
///
/// Injected by the frontend (composition root) to perform
/// filesystem-level validation. The view model calls this
/// during validation but does not depend on `std::fs`.
pub trait StoragePathValidator: std::fmt::Debug {
    fn validate(&self, path: &Path) -> Result<(), String>;
}

use super::{
    ValidationState,
    projection::ProjectionHub,
    property::{ObservablePropertyInner, ReadOnlyObservableProperty},
};

/// Error type for view model operations.
#[derive(Debug, thiserror::Error)]
pub enum ViewModelError {
    #[error("settings mutation is not allowed during property notification")]
    ReentrantMutation,
    #[error("unknown system: {0}")]
    UnknownSystem(String),
    #[error("unknown controller slot: {0}")]
    UnknownSlot(String),
    #[error("unknown controller profile: {0}")]
    UnknownController(String),
    #[error("invalid system settings choice")]
    InvalidSystemChoice,
    #[error("capture target is not available in the current topology")]
    InvalidCaptureTarget,
}

/// Mutable state for the settings editor.
#[derive(Clone)]
pub struct EditorState {
    pub initial: SettingsSnapshot,
    pub draft: SettingsSnapshot,
    pub capture_target: Option<CaptureTarget>,
    pub validation: ValidationState,
    pub revision: u64,
    pub(crate) catalog: FactoryCatalog,
    pub supported_sample_rates: Arc<[u32]>,
    pub storage_validator: Option<Rc<dyn StoragePathValidator>>,
}

/// Lightweight handle to the shared editor state and projection hub.
///
/// All mutations go through [`SettingsEditor::transact()`].
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
    ) -> Self {
        let current = Rc::new(RefCell::new(EditorState {
            initial: snapshot.clone(),
            draft: snapshot,
            capture_target: None,
            validation: ValidationState { issues: vec![] },
            revision: 0,
            catalog,
            supported_sample_rates,
            storage_validator: None,
        }));

        let revision_inner = Rc::new(ObservablePropertyInner::new(0u64));

        Self {
            current,
            projections: ProjectionHub::new(),
            validator: Rc::new(|_| ValidationState { issues: vec![] }),
            notifying: Rc::new(Cell::new(false)),
            revision_inner,
        }
    }

    pub fn set_storage_validator(&self, validator: Box<dyn StoragePathValidator>) {
        self.current.borrow_mut().storage_validator = Some(Rc::from(validator));
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
        self.current.borrow().draft.clone()
    }

    pub(crate) fn registry(&self) -> std::cell::Ref<'_, EditorState> {
        self.current.borrow()
    }

    pub fn set_validator(&mut self, validator: impl Fn(&EditorState) -> ValidationState + 'static) {
        let result = validator(&self.current.borrow());
        self.validator = Rc::new(validator);
        self.current.borrow_mut().validation = result;
    }

    pub(crate) fn projections(&self) -> &ProjectionHub {
        &self.projections
    }

    /// Execute a domain mutation within a clone-on-write transaction.
    ///
    /// 1. Clones current state to `candidate`
    /// 2. Applies `mutate` to `candidate`
    /// 3. Re-validates and advances revision
    /// 4. Prepares all projections from `candidate`
    /// 5. Silent-applies prepared projections
    /// 6. Notifies callbacks
    pub fn transact<R>(
        &self,
        mutate: impl FnOnce(&mut EditorState) -> Result<R, ViewModelError>,
    ) -> Result<R, ViewModelError> {
        if self.notifying.get() {
            return Err(ViewModelError::ReentrantMutation);
        }

        let original = self.current.borrow().clone();
        let mut candidate = original.clone();
        let result = mutate(&mut candidate)?;

        if candidate.draft == original.draft && candidate.capture_target == original.capture_target
        {
            return Ok(result);
        }

        candidate.validation = (self.validator)(&candidate);
        candidate.revision = original.revision + 1;
        let rev_value = candidate.revision;
        let prepared = self.projections.prepare_all(&candidate);

        *self.current.borrow_mut() = candidate;

        // Apply prepared projections silently; collect notification
        // closures to invoke after all values are in place.
        let mut notifications: Vec<Box<dyn FnOnce()>> = Vec::new();
        for projection in prepared {
            if let Some(notify) = projection.apply() {
                notifications.push(notify);
            }
        }

        #[cfg(debug_assertions)]
        self.projections.assert_all_synced(&self.current.borrow());

        // Collect revision observer callbacks
        let rev_callbacks = self.revision_inner.set(rev_value);

        // Fire all notifications: first projection observers, then revision
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
            Ok(state.draft.clone())
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
    use nerust_gui_settings::{
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
        let catalog = crate::settings::catalog::FactoryCatalog::new(Vec::new());
        SettingsEditor::new(empty_snapshot(), catalog, Arc::new([]))
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
    fn transact_mutation_advances_revision() {
        let editor = test_editor();

        let result: Result<(), ViewModelError> = editor.transact(|state| {
            state.draft.local.audio.muted = true;
            Ok(())
        });
        assert!(result.is_ok());
        assert_eq!(editor.revision(), 1);
    }

    #[test]
    fn transact_error_does_not_change_state() {
        let editor = test_editor();

        let result: Result<(), ViewModelError> = editor.transact(|state| {
            state.draft.local.audio.muted = true;
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
        // Manually set validation to invalid
        editor.current.borrow_mut().validation = ValidationState {
            issues: vec![super::super::ValidationIssue {
                scope: super::super::ValidationScope::Persistence,
                message: "test error".into(),
            }],
        };
        assert!(editor.finish().is_err());
    }

    #[test]
    fn projection_observer_fires_after_transaction() {
        use super::super::property::ReadOnlyObservableProperty;
        use std::rc::Rc;

        let editor = test_editor();

        // Register a projection via the hub
        let prop: ReadOnlyObservableProperty<bool> =
            editor
                .projections()
                .register("test_proj", false, |state| state.draft.local.audio.muted);

        // Observe the projection
        let observed = Rc::new(Cell::new(false));
        let observed_cb = Rc::clone(&observed);
        let _sub = prop.observe(move |v| {
            observed_cb.set(*v);
        });

        // Mutate via transact — projection observer should fire
        editor
            .transact(|state| {
                state.draft.local.audio.muted = true;
                Ok(())
            })
            .unwrap();

        assert!(
            observed.get(),
            "projection observer should have fired after transact"
        );
    }

    #[test]
    fn set_validator_runs_initial_validation() {
        let mut editor = test_editor();
        // Initial validation should be empty (default snapshot is valid)
        assert!(editor.current().validation.can_submit());

        // Set a validator that flags persistence issues
        editor.set_validator(|state| {
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
        });

        // set_validator should run validation immediately
        assert!(!editor.current().validation.can_submit());
    }

    #[test]
    fn noop_mutation_skips_validation() {
        let editor = test_editor();
        // Set up an invalid state directly
        editor.current.borrow_mut().validation = ValidationState {
            issues: vec![super::super::ValidationIssue {
                scope: super::super::ValidationScope::Persistence,
                message: "test".into(),
            }],
        };
        // A no-op transact should short-circuit without re-validating
        // But since validation happens inside transact(), the error persists
        let result: Result<(), ViewModelError> = editor.transact(|_| Ok(()));
        assert!(result.is_ok());
        assert!(!editor.current().validation.can_submit());
    }
}
