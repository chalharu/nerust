#![allow(dead_code)]

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    sync::Arc,
};

use nerust_gui_settings::snapshot::SettingsSnapshot;
use nerust_settings_core::editor::CaptureTarget;

use super::{
    ValidationState,
    catalog::FactoryCatalog,
    projection::ProjectionHub,
    property::{ObservablePropertyInner, ReadOnlyObservableProperty},
};

/// Storage path validation error.
#[derive(Debug, Clone)]
pub enum StoragePathError {
    NotDirectory,
    Inaccessible(String),
}

impl std::fmt::Display for StoragePathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoragePathError::NotDirectory => write!(f, "not a directory"),
            StoragePathError::Inaccessible(msg) => write!(f, "{msg}"),
        }
    }
}

/// Storage path validation port — injected by the composition root.
pub trait StoragePathValidator: std::fmt::Debug {
    fn validate(&self, path: &Path) -> Result<(), StoragePathError>;
}

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

/// No-op validator for use in tests.
#[cfg(test)]
#[derive(Debug)]
pub struct NoopStoragePathValidator;
#[cfg(test)]
impl StoragePathValidator for NoopStoragePathValidator {
    fn validate(&self, _path: &Path) -> Result<(), StoragePathError> {
        Ok(())
    }
}

/// Mutable state for the settings editor.
#[derive(Clone)]
pub struct EditorState {
    pub(crate) draft: SettingsSnapshot,
    pub(crate) capture_target: Option<CaptureTarget>,
    pub validation: ValidationState,
    pub revision: u64,
    pub(crate) catalog: FactoryCatalog,
    pub(crate) supported_sample_rates: Arc<[u32]>,
    pub(crate) storage_validator: Rc<dyn StoragePathValidator>,
    /// Tracks which validation scopes have changed since last revalidation.
    /// When `None`, all scopes need revalidation (initial state or forced).
    pub(crate) dirty_scopes: Option<Vec<super::ValidationScope>>,
}

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
        let editor_state = EditorState {
            draft: snapshot,
            capture_target: None,
            validation: ValidationState { issues: vec![] },
            revision: 0,
            catalog,
            supported_sample_rates,
            storage_validator,
            dirty_scopes: None,
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

    /// Mark the given scopes as needing revalidation.
    /// Passing `None` forces full revalidation.
    pub fn mark_dirty(&self, scopes: Option<Vec<super::ValidationScope>>) {
        let mut state = self.current.borrow_mut();
        match (&mut state.dirty_scopes, scopes) {
            (_, None) => state.dirty_scopes = None,
            (None, Some(_)) => {} // already full
            (Some(existing), Some(new)) => existing.extend(new),
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
        self.current.borrow().draft.clone()
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

        let original = self.current.borrow().clone();
        let mut candidate = original.clone();
        let result = mutate(&mut candidate)?;

        if candidate.draft == original.draft
            && candidate.capture_target == original.capture_target
            && candidate.dirty_scopes == original.dirty_scopes
        {
            return Ok(result);
        }

        // Re-validate: currently full revalidation. When dirty_scopes is Some
        // and narrowed, this can be changed to scoped validation.
        candidate.validation = (self.validator)(&candidate);
        candidate.dirty_scopes = None;
        candidate.revision = original.revision + 1;
        let rev_value = candidate.revision;
        let prepared = self.projections.prepare_all(&candidate);

        *self.current.borrow_mut() = candidate;

        let mut notifications: Vec<Box<dyn FnOnce()>> = Vec::new();
        for projection in prepared {
            if let Some(notify) = projection.apply() {
                notifications.push(notify);
            }
        }

        #[cfg(debug_assertions)]
        self.projections.assert_all_synced(&self.current.borrow());

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
        let catalog = crate::settings::catalog::FactoryCatalog::new(Vec::new());
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
                state.draft.local.audio.muted = true;
                Ok(())
            })
            .unwrap();

        assert!(
            observed.get(),
            "projection observer should have fired after transact"
        );
    }
}
