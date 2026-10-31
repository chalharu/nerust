//! Toolkit-independent settings dialog state and transitions.
//!
//! The view model is the sole settings draft. Native adapters render its
//! projections and execute returned effects, feeding their results back here.

use std::{
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_runtime::settings::{
    SettingsSnapshot,
    apply::{DirectoryValidationError, validate_directory_path_typed},
};
use nerust_gui_shell::registry::SystemRegistry;
use nerust_gui_viewmodel::settings::{
    SettingsViewModel, StoragePathError, StoragePathValidator, Subscription, ViewModelError,
    dto::ChoiceView,
};
use nerust_input_traits::AttachmentId;
use nerust_keyboard::Key;
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::{UiText, text as ui_text},
    model::{
        language::AppLanguage,
        local::{NotificationAreaMode, NotificationColor, NotificationPosition, ScalingMode},
        shared::StoragePolicy,
    },
};
use nerust_system_traits::factory::{
    CoreFactory,
    descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId},
};

use crate::debugger::lock_recovered;

mod screen;
pub use screen::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    General,
    Input,
    Video,
    Audio,
    Notifications,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPageSection {
    Attachment(usize),
}

/// The trailing input tab contains global, rather than machine, shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputTab {
    System(usize),
    Shortcuts,
}

#[derive(Debug, Clone)]
pub enum SettingsEvent {
    SelectPage(SettingsPage),
    SelectInputSection(InputPageSection),
    SelectSystemTab(usize),
    SelectInputTab(usize),
    SetLanguage(ChoiceView<AppLanguage>),
    SetStoragePolicy(ChoiceView<StoragePolicy>),
    SetStorageDirectory(String),
    BrowseStorageDirectory,
    StorageDirectorySelected(Option<PathBuf>),
    ToggleFullscreenDefault(bool),
    SetScaling(ChoiceView<ScalingMode>),
    ToggleVsync(bool),
    ToggleMute(bool),
    SetVolume(u8),
    SetSampleRate(ChoiceView<u32>),
    SetLatency(u16),
    ToggleInfoEnabled(bool),
    SetInfoDuration(u32),
    SetInfoColor(ChoiceView<NotificationColor>),
    ToggleWarningEnabled(bool),
    SetWarningDuration(u32),
    SetWarningColor(ChoiceView<NotificationColor>),
    ToggleErrorEnabled(bool),
    ToggleErrorModal(bool),
    SetErrorColor(ChoiceView<NotificationColor>),
    ToggleStatusEnabled(bool),
    SetPosition(ChoiceView<NotificationPosition>),
    SetStatusPosition(ChoiceView<NotificationPosition>),
    SetAreaMode(ChoiceView<NotificationAreaMode>),
    SetStatusAreaMode(ChoiceView<NotificationAreaMode>),
    SetSystemChoice(String, ChoiceView<SystemSettingsChoiceId>),
    StartCapture(CaptureTarget),
    ClearCapture(CaptureTarget),
    CaptureKey(Key),
    SetControllerSlot {
        slot: AttachmentId,
        controller_id: Option<String>,
    },
    Submit,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsEffect {
    BrowseStorageDirectory { title: String },
}

/// Internal signals, not a second settings draft. Clones refer to
/// the controller's same close/apply/repaint mailboxes.
#[derive(Clone)]
pub(crate) struct SettingsMailbox {
    pub(crate) should_close: Arc<AtomicBool>,
    pub(crate) pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
    pub(crate) view_invalidated: Arc<AtomicBool>,
}

impl SettingsMailbox {
    pub(crate) fn take_pending_apply(&self) -> Option<SettingsSnapshot> {
        lock_recovered(&self.pending_apply).take()
    }
}

pub struct SettingsController {
    pub(crate) mailbox: SettingsMailbox,
    pub vm: SettingsViewModel,
    _revision_subscription: Subscription,
    pub page: SettingsPage,
    pub system_tab_index: Option<usize>,
    pub input_tab: Option<InputTab>,
    pub input_section: InputPageSection,
    pub storage_directory_input: String,
    pub error_message: Option<String>,
}

fn system_id_key(factory: &Arc<dyn CoreFactory>) -> String {
    factory.system_id().to_string()
}

/// First key occurring twice, in encounter order.
fn first_duplicate_key<T>(
    items: impl IntoIterator<Item = T>,
    mut key: impl FnMut(&T) -> String,
) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .map(|item| key(&item))
        .find(|id| !seen.insert(id.clone()))
}

/// First duplicate SystemId in registration order, if any.
pub fn find_duplicate_system_id(registry: &SystemRegistry) -> Option<String> {
    first_duplicate_key(registry.all(), |factory| system_id_key(factory))
}

fn deduped_by_key<T>(
    items: impl IntoIterator<Item = T>,
    mut key: impl FnMut(&T) -> String,
    mut on_duplicate: impl FnMut(&T),
) -> Vec<T> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| {
            if seen.insert(key(item)) {
                true
            } else {
                on_duplicate(item);
                false
            }
        })
        .collect()
}

fn deduped_factories(registry: &SystemRegistry) -> Vec<Arc<dyn CoreFactory>> {
    deduped_by_key(registry.all().iter().cloned(), system_id_key, |factory| {
        log::warn!(
            "ignoring factory '{}' with duplicate SystemId",
            factory.display_name()
        );
    })
}

#[derive(Debug)]
pub struct FsStoragePathValidator;

impl StoragePathValidator for FsStoragePathValidator {
    fn validate(&self, path: &std::path::Path) -> Result<(), StoragePathError> {
        match validate_directory_path_typed(path) {
            Ok(()) => Ok(()),
            Err(DirectoryValidationError::NotDirectory) => Err(StoragePathError::NotDirectory),
            Err(DirectoryValidationError::Inaccessible(e)) => {
                Err(StoragePathError::Inaccessible(e.to_string()))
            }
        }
    }
}

impl SettingsController {
    pub fn new(
        snapshot: &SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
    ) -> Self {
        Self::new_with_shared(
            snapshot,
            registry,
            audio_registry,
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
            Arc::new(AtomicBool::new(false)),
        )
    }

    fn new_with_shared(
        snapshot: &SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
        should_close: Arc<AtomicBool>,
        pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
        view_invalidated: Arc<AtomicBool>,
    ) -> Self {
        let supported_sample_rates: Arc<[u32]> = {
            let rates = audio_registry.supported_rates();
            if rates.is_empty() {
                Arc::new([44_100, 48_000])
            } else {
                rates.iter().copied().collect()
            }
        };
        let vm = SettingsViewModel::new(
            snapshot.clone(),
            deduped_factories(&registry),
            supported_sample_rates,
            Rc::new(FsStoragePathValidator),
        )
        .expect("deduped factory catalog contains a duplicate SystemId");
        let invalidated = Arc::clone(&view_invalidated);
        let revision_subscription = vm.revision.observe(move |_| {
            invalidated.store(true, Ordering::Release);
        });
        let has_systems = !vm.systems().is_empty();
        Self {
            mailbox: SettingsMailbox {
                should_close,
                pending_apply,
                view_invalidated,
            },
            vm,
            _revision_subscription: revision_subscription,
            page: SettingsPage::General,
            system_tab_index: has_systems.then_some(0),
            input_tab: has_systems.then_some(InputTab::System(0)),
            input_section: InputPageSection::Attachment(0),
            storage_directory_input: snapshot
                .shared
                .persistence
                .storage_directory
                .as_ref()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default(),
            error_message: None,
        }
    }

    pub fn language(&self) -> AppLanguage {
        self.vm.general.view.get().language
    }

    pub fn validation_errors(&self) -> Vec<String> {
        self.vm
            .finish()
            .err()
            .map(|v| v.issues.into_iter().map(|i| i.message).collect())
            .unwrap_or_default()
    }

    /// Shared validation banner: headline is the first issue, storage
    /// shows the first persistence-scope issue. Replaces the former
    /// hardcoded storage message with the localized validator output.
    pub fn validation_banner(&self) -> nerust_gui_viewmodel::settings::ValidationBanner {
        use nerust_gui_viewmodel::settings::ValidationBanner;
        match self.vm.finish() {
            Ok(_) => ValidationBanner {
                can_submit: true,
                headline: None,
                storage: None,
            },
            Err(validation) => validation.banner(),
        }
    }

    pub fn storage_error(&self) -> Option<String> {
        self.validation_banner().storage
    }

    /// Selection indices deliberately retain the original adapter's permissive
    /// behavior: an out-of-range system tab renders an empty page, not a clamp.
    pub fn update(&mut self, event: SettingsEvent) -> Option<SettingsEffect> {
        use SettingsEvent as E;
        let before = (
            self.page,
            self.system_tab_index,
            self.input_tab,
            self.input_section,
            self.storage_directory_input.clone(),
            self.error_message.clone(),
        );
        self.error_message = None;
        let mut effect = None;
        match event {
            E::SelectPage(page) => self.page = page,
            E::SelectInputSection(section) => self.input_section = section,
            E::SelectSystemTab(index) => self.system_tab_index = Some(index),
            E::SelectInputTab(index) => {
                self.input_tab = Some(if index == self.vm.inputs().len() {
                    InputTab::Shortcuts
                } else {
                    InputTab::System(index)
                });
                self.input_section = InputPageSection::Attachment(0);
            }
            E::SetLanguage(choice) => self.err(self.vm.general.set_language(choice.value)),
            E::SetStoragePolicy(choice) => {
                self.err(self.vm.general.set_storage_policy(choice.value));
            }
            E::SetStorageDirectory(value) => self.set_storage_directory(value),
            E::BrowseStorageDirectory => {
                effect = Some(SettingsEffect::BrowseStorageDirectory {
                    title: ui_text(self.language(), UiText::SaveStorageDirectory).into(),
                });
            }
            E::StorageDirectorySelected(path) => {
                if let Some(path) = path {
                    self.set_storage_directory(path.to_string_lossy().to_string());
                }
            }
            E::ToggleFullscreenDefault(value) => {
                self.err(self.vm.video.set_fullscreen_default(value));
            }
            E::SetScaling(choice) => self.err(self.vm.video.set_scaling(choice.value)),
            E::ToggleVsync(value) => self.err(self.vm.video.set_vsync(value)),
            E::ToggleMute(value) => self.err(self.vm.audio.set_mute(value)),
            E::SetVolume(value) => self.err(self.vm.audio.set_volume(value)),
            E::SetSampleRate(choice) => self.err(self.vm.audio.set_sample_rate(choice.value)),
            E::SetLatency(value) => self.err(self.vm.audio.set_latency(value)),
            E::ToggleInfoEnabled(value) => self.err(self.vm.notifications.set_info_enabled(value)),
            E::SetInfoDuration(value) => {
                self.err(self.vm.notifications.set_info_duration_ms(value))
            }
            E::SetInfoColor(choice) => self.err(self.vm.notifications.set_info_color(choice.value)),
            E::ToggleWarningEnabled(value) => {
                self.err(self.vm.notifications.set_warning_enabled(value));
            }
            E::SetWarningDuration(value) => {
                self.err(self.vm.notifications.set_warning_duration_ms(value));
            }
            E::SetWarningColor(choice) => {
                self.err(self.vm.notifications.set_warning_color(choice.value));
            }
            E::ToggleErrorEnabled(value) => {
                self.err(self.vm.notifications.set_error_enabled(value))
            }
            E::ToggleErrorModal(value) => self.err(self.vm.notifications.set_error_modal(value)),
            E::SetErrorColor(choice) => {
                self.err(self.vm.notifications.set_error_color(choice.value))
            }
            E::ToggleStatusEnabled(value) => {
                self.err(self.vm.notifications.set_status_enabled(value));
            }
            E::SetPosition(choice) => self.err(self.vm.notifications.set_position(choice.value)),
            E::SetStatusPosition(choice) => {
                self.err(self.vm.notifications.set_status_position(choice.value));
            }
            E::SetAreaMode(choice) => self.err(self.vm.notifications.set_area_mode(choice.value)),
            E::SetStatusAreaMode(choice) => {
                self.err(self.vm.notifications.set_status_area_mode(choice.value));
            }
            E::SetSystemChoice(field, choice) => self.set_system_choice(field, choice),
            E::SetControllerSlot {
                slot,
                controller_id,
            } => {
                self.set_controller_slot(slot, controller_id);
            }
            E::StartCapture(target) => self.err(self.vm.capture.start_capture(target)),
            E::ClearCapture(target) => self.err(self.vm.capture.clear_binding(&target)),
            E::CaptureKey(key) => self.vm.capture.apply_captured_key(key),
            E::Submit => {
                if let Ok(snapshot) = self.vm.finish() {
                    *lock_recovered(&self.mailbox.pending_apply) = Some(snapshot);
                    self.mailbox.should_close.store(true, Ordering::Release);
                }
            }
            E::Cancel => self.mailbox.should_close.store(true, Ordering::Release),
        }
        let after = (
            self.page,
            self.system_tab_index,
            self.input_tab,
            self.input_section,
            self.storage_directory_input.clone(),
            self.error_message.clone(),
        );
        // VM mutations invalidate through the subscription. Local edits and
        // error transitions must also repaint when the VM revision is unchanged.
        if before != after {
            self.mailbox.view_invalidated.store(true, Ordering::Release);
        }
        effect
    }

    fn err(&mut self, result: Result<(), ViewModelError>) {
        if let Err(error) = result {
            self.error_message = Some(error.to_string());
        }
    }

    fn set_storage_directory(&mut self, value: String) {
        self.storage_directory_input = value.clone();
        self.err(
            self.vm
                .general
                .set_storage_directory((!value.is_empty()).then(|| value.into())),
        );
    }

    fn set_system_choice(&mut self, field: String, choice: ChoiceView<SystemSettingsChoiceId>) {
        if let Some(index) = self.system_tab_index
            && let Some(vm) = self.vm.systems().get(index)
        {
            self.err(vm.set_choice(&SystemSettingsFieldId(field.into()), &choice.value));
        }
    }

    fn set_controller_slot(&mut self, slot: AttachmentId, controller_id: Option<String>) {
        if let Some(InputTab::System(index)) = self.input_tab
            && let Some(vm) = self.vm.inputs().get(index)
        {
            self.err(vm.set_controller_slot(slot, controller_id.as_deref()));
        }
    }
}

#[cfg(test)]
mod tests;
