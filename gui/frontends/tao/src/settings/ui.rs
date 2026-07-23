use std::{
    fmt,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use iced::{
    Font, Length, Task, Theme,
    alignment::Alignment,
    widget::{
        button, checkbox, column, container, pick_list, radio, row, scrollable, slider, text,
        text_input,
    },
};
use iced_winit::program::Program;
use nerust_core_traits::{
    audio::AudioBackendRegistry,
    factory::descriptor::{SystemSettingsFieldKind, SystemSettingsFieldModel},
};
use nerust_gui_runtime::settings::{SettingsSnapshot, apply::validate_shared_settings};
use nerust_gui_settings::{language::AppLanguage, local::ScalingMode, shared::StoragePolicy};
use nerust_gui_shell::{
    registry::SystemRegistry,
    settings::{
        bindings::{
            conflicting_keys,
            descriptors::{keyboard_binding_sections, shortcut_descriptors},
        },
        editor::{CaptureTarget, apply_capture_target, current_binding_label},
        factory::{apply_settings_choice, resolve_label, settings_view},
        i18n::{UiText, text as ui_text},
    },
};
use nerust_input_traits::{
    AttachmentId, ControllerProfile, InputAssignments, InputTopologyDescriptor,
};
use nerust_keyboard::Key;
use rfd::FileDialog;

type El<'a> = iced::Element<'a, Message, iced::Theme, iced_tiny_skia::Renderer>;
type PendingAssignments =
    Rc<Mutex<Option<Vec<(nerust_core_traits::identity::SystemId, InputAssignments)>>>>;

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Choice<T: Clone + Eq> {
    value: T,
    label: String,
}

impl<T: Clone + Eq> fmt::Display for Choice<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsPage {
    General,
    Input,
    Video,
    Audio,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputPageSection {
    Attachment(usize),
    Shortcuts,
}

#[derive(Debug, Clone)]
pub(crate) enum Message {
    SelectPage(SettingsPage),
    SelectInputSection(InputPageSection),
    SelectSystemTab(usize),
    SelectInputTab(usize),
    SetLanguage(Choice<AppLanguage>),
    SetStoragePolicy(Choice<StoragePolicy>),
    SetStorageDirectory(String),
    BrowseStorageDirectory,
    ToggleFullscreenDefault(bool),
    SetScaling(Choice<ScalingMode>),
    ToggleVsync(bool),
    ToggleMute(bool),
    SetVolume(u8),
    SetSampleRate(Choice<u32>),
    SetLatency(u16),
    SetSystemChoice(String, Choice<String>),
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

pub(crate) struct SettingsAppProgram {
    pub(crate) snapshot: SettingsSnapshot,
    pub(crate) registry: Arc<SystemRegistry>,
    pub(crate) audio_registry: Arc<AudioBackendRegistry>,
    pub(crate) should_close: Arc<AtomicBool>,
    pub(crate) pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
    pub(crate) pending_assignments: PendingAssignments,
    pub(crate) capture_target: Arc<Mutex<Option<CaptureTarget>>>,
}

impl Program for SettingsAppProgram {
    type State = SettingsAppState;
    type Message = Message;
    type Theme = Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "nerust_settings"
    }

    fn settings(&self) -> iced::Settings {
        iced::Settings {
            default_font: default_font(),
            default_text_size: iced::Pixels(16.0),
            ..Default::default()
        }
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        let state = SettingsAppState::new_with_shared(
            &self.snapshot,
            self.registry.clone(),
            self.audio_registry.clone(),
            self.should_close.clone(),
            self.pending_apply.clone(),
            self.pending_assignments.clone(),
            self.capture_target.clone(),
        );
        (state, Task::none())
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        state.update(message)
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        state.view()
    }

    fn window(&self) -> Option<iced::window::Settings> {
        None
    }
}

// ---------------------------------------------------------------------------
// SettingsAppState
// ---------------------------------------------------------------------------

pub(crate) struct SettingsAppState {
    pub(crate) should_close: Arc<AtomicBool>,
    pub(crate) pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
    pub(crate) pending_assignments: PendingAssignments,
    pub(crate) capture_target: Arc<Mutex<Option<CaptureTarget>>>,
    registry: Arc<SystemRegistry>,
    audio_registry: Arc<AudioBackendRegistry>,
    draft: SettingsSnapshot,
    controller_assignments: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)>,
    /// Snapshot of initial assignments for change detection at Submit time.
    initial_assignments_pairs: Vec<(String, Option<String>)>,
    page: SettingsPage,
    system_tab_index: Option<usize>,
    input_tab_index: Option<usize>,
    input_section: InputPageSection,
    storage_directory_input: String,
    error_message: Option<String>,
}

impl SettingsAppState {
    pub(crate) fn new(
        snapshot: &SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
    ) -> Self {
        let storage_directory_input = snapshot
            .shared
            .persistence
            .storage_directory
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();
        let (controller_assignments, initial_assignments_pairs, has_systems) =
            if let Some(factory) = registry.all().first() {
                let sid = factory.system_id().to_string();
                let input_factory = factory.input_system_factory();
                let default_assignments = input_factory.default_assignments();
                let assignments: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)> = snapshot
                    .app_state
                    .controller_assignments
                    .get(&sid)
                    .map(|pairs| {
                        pairs
                            .iter()
                            .filter_map(|(slot_id, ctrl_opt)| {
                                let att = match input_factory.resolve_slot(slot_id) {
                                    Some(a) => a,
                                    None => {
                                        log::warn!(
                                            "unknown persisted slot ID in settings: {slot_id}"
                                        );
                                        return None;
                                    }
                                };
                                let profile = ctrl_opt
                                    .as_ref()
                                    .and_then(|id| input_factory.resolve_controller(id));
                                Some((att, profile))
                            })
                            .collect()
                    })
                    .unwrap_or_else(|| {
                        default_assignments
                            .slots
                            .iter()
                            .map(|(slot_id, profile)| (*slot_id, profile.clone()))
                            .collect()
                    });
                let pairs: Vec<(String, Option<String>)> = assignments
                    .iter()
                    .map(|(s, c)| {
                        (
                            s.as_str().to_string(),
                            c.as_ref().map(|p| p.profile_id().to_string()),
                        )
                    })
                    .collect();
                (assignments.clone(), pairs, true)
            } else {
                (vec![], vec![], false)
            };
        Self {
            should_close: Arc::new(AtomicBool::new(false)),
            pending_apply: Arc::new(Mutex::new(None)),
            pending_assignments: Rc::new(Mutex::new(None)),
            capture_target: Arc::new(Mutex::new(None)),
            controller_assignments,
            initial_assignments_pairs,
            registry,
            audio_registry,
            draft: snapshot.clone(),
            page: SettingsPage::General,
            system_tab_index: if has_systems { Some(0) } else { None },
            input_tab_index: if has_systems { Some(0) } else { None },
            input_section: InputPageSection::Attachment(0),
            storage_directory_input,
            error_message: None,
        }
    }

    pub(crate) fn new_with_shared(
        snapshot: &SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
        should_close: Arc<AtomicBool>,
        pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
        pending_assignments: PendingAssignments,
        capture_target: Arc<Mutex<Option<CaptureTarget>>>,
    ) -> Self {
        let mut state = Self::new(snapshot, registry, audio_registry);
        state.should_close = should_close;
        state.pending_apply = pending_apply;
        state.pending_assignments = pending_assignments;
        state.capture_target = capture_target;
        state
    }

    fn language(&self) -> AppLanguage {
        self.draft.shared.general.language
    }

    fn validation_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if let Err(error) = validate_shared_settings(&self.draft.shared) {
            errors.push(error.to_string());
        }
        if self.registry.all().is_empty() {
            return errors;
        }
        if !self.controller_assignments.iter().any(|(_, c)| c.is_some()) {
            errors.push("At least one controller must be assigned".to_string());
        }
        let Some(factory) = self
            .input_tab_index
            .and_then(|i| self.registry.all().get(i))
        else {
            return errors;
        };
        for (key, labels) in conflicting_keys(
            &self.draft.shared,
            &input_topology(self),
            factory.system_id(),
        ) {
            errors.push(format!("{}: {}", key.label(), labels.join(", ")));
        }
        errors
    }

    fn storage_error(&self) -> Option<String> {
        validate_shared_settings(&self.draft.shared)
            .err()
            .map(|error| error.to_string())
    }

    fn input_conflict(&self) -> Option<String> {
        let input_tab_index = self.input_tab_index?;
        let system_id = self.registry.all().get(input_tab_index)?.system_id();
        let (key, labels) = conflicting_keys(&self.draft.shared, &input_topology(self), system_id)
            .into_iter()
            .next()?;
        Some(format!("{}: {}", key.label(), labels.join(", ")))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        self.error_message = None;
        match message {
            Message::SelectPage(page) => self.page = page,
            Message::SelectInputSection(section) => self.input_section = section,
            Message::SelectSystemTab(index) => self.system_tab_index = Some(index),
            Message::SelectInputTab(index) => self.input_tab_index = Some(index),
            Message::SetLanguage(choice) => self.draft.shared.general.language = choice.value,
            Message::SetStoragePolicy(choice) => {
                self.draft.shared.persistence.storage_policy = choice.value;
            }
            Message::SetStorageDirectory(value) => {
                self.storage_directory_input = value;
                self.draft.shared.persistence.storage_directory =
                    (!self.storage_directory_input.is_empty())
                        .then(|| self.storage_directory_input.clone().into());
            }
            Message::BrowseStorageDirectory => {
                if let Some(path) = FileDialog::new()
                    .set_title(ui_text(self.language(), UiText::SaveStorageDirectory))
                    .pick_folder()
                {
                    let path = path.to_string_lossy().to_string();
                    self.storage_directory_input = path.clone();
                    self.draft.shared.persistence.storage_directory = Some(path.into());
                }
            }
            Message::ToggleFullscreenDefault(value) => {
                self.draft.local.video.window.fullscreen_default = value;
            }
            Message::SetScaling(choice) => self.draft.local.video.window.scaling = choice.value,
            Message::ToggleVsync(value) => self.draft.local.video.presentation.vsync = value,
            Message::ToggleMute(value) => self.draft.local.audio.muted = value,
            Message::SetVolume(value) => self.draft.local.audio.master_volume_percent = value,
            Message::SetSampleRate(choice) => self.draft.local.audio.sample_rate = choice.value,
            Message::SetLatency(value) => self.draft.local.audio.latency_ms = value,
            Message::SetSystemChoice(field, choice) => {
                if let Some(factory) = self
                    .system_tab_index
                    .and_then(|i| self.registry.all().get(i))
                {
                    let _ = apply_settings_choice(
                        factory.as_ref(),
                        &mut self.draft,
                        &nerust_core_traits::factory::descriptor::SystemSettingsFieldId(
                            field.into(),
                        ),
                        &nerust_core_traits::factory::descriptor::SystemSettingsChoiceId(
                            choice.value.into(),
                        ),
                    );
                }
            }
            Message::SetControllerSlot {
                slot,
                controller_id,
            } => {
                let Some(factory) = self
                    .input_tab_index
                    .and_then(|i| self.registry.all().get(i))
                else {
                    return Task::none();
                };
                let input_factory = factory.input_system_factory();
                let profile = controller_id.as_ref().and_then(|id| {
                    input_factory
                        .controllers()
                        .iter()
                        .find(|p| p.profile_id().as_str() == id)
                        .cloned()
                });
                // For multi-port controllers (port_set with >1 port),
                // clear other occupied slots in the same set.
                if let Some(ref p) = profile {
                    for ps in p.port_sets() {
                        if ps.ports.len() <= 1 {
                            continue;
                        }
                        if !ps.ports.contains(&slot) {
                            continue;
                        }
                        for &port in ps.ports {
                            if port != slot
                                && let Some(other) = self
                                    .controller_assignments
                                    .iter_mut()
                                    .find(|(s, _)| *s == port)
                            {
                                other.1 = None;
                            }
                        }
                    }
                }
                if let Some(entry) = self
                    .controller_assignments
                    .iter_mut()
                    .find(|(s, _)| *s == slot)
                {
                    entry.1 = profile.clone();
                }
                // Keep unassigned slots empty (allow disconnected ports).
                // Sync to draft.app_state for persistence
                let sid = factory.system_id().to_string();
                self.draft.app_state.controller_assignments.insert(
                    sid,
                    self.controller_assignments
                        .iter()
                        .map(|(s, c)| {
                            (
                                s.to_string(),
                                c.as_ref().map(|p| p.profile_id().to_string()),
                            )
                        })
                        .collect(),
                );
            }
            Message::StartCapture(target) => {
                *self.capture_target.lock().unwrap() = Some(target);
            }
            Message::ClearCapture(target) => {
                apply_capture_target(&mut self.draft, &target, None);
                self.capture_target.lock().unwrap().take();
            }
            Message::CaptureKey(key) => {
                let target = self.capture_target.lock().unwrap().take();
                if let Some(target) = target {
                    apply_capture_target(&mut self.draft, &target, Some(key));
                }
            }
            Message::Submit => {
                if !self.validation_errors().is_empty() {
                    return Task::none();
                }
                *self.pending_apply.lock().unwrap() = Some(self.draft.clone());
                // Only push assignments if they actually changed
                let new_pairs: Vec<(String, Option<String>)> = self
                    .controller_assignments
                    .iter()
                    .map(|(s, c)| {
                        (
                            s.to_string(),
                            c.as_ref().map(|p| p.profile_id().to_string()),
                        )
                    })
                    .collect();
                if new_pairs != self.initial_assignments_pairs {
                    let Some(factory) = self
                        .input_tab_index
                        .and_then(|i| self.registry.all().get(i))
                    else {
                        return Task::none();
                    };
                    let sid = factory.system_id();
                    *self.pending_assignments.lock().unwrap() = Some(vec![(
                        sid,
                        InputAssignments {
                            slots: self
                                .controller_assignments
                                .iter()
                                .map(|(s, c)| (*s, c.clone()))
                                .collect(),
                        },
                    )]);
                }
                self.should_close.store(true, Ordering::Release);
            }
            Message::Cancel => {
                self.should_close.store(true, Ordering::Release);
            }
        }
        Task::none()
    }

    fn view(&self) -> El<'_> {
        let language = self.language();
        let validation_errors = self.validation_errors();
        let can_submit = validation_errors.is_empty();

        let sidebar = column![
            page_radio(language, UiText::General, SettingsPage::General, self.page),
            page_radio(language, UiText::Input, SettingsPage::Input, self.page),
            page_radio(language, UiText::Video, SettingsPage::Video, self.page),
            page_radio(language, UiText::Audio, SettingsPage::Audio, self.page),
            page_radio(language, UiText::System, SettingsPage::System, self.page),
        ]
        .spacing(10)
        .width(Length::Shrink);

        let content = scrollable(
            container(self.page_content())
                .padding(12)
                .width(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill);

        let mut root = column![
            row![
                container(sidebar).padding(12).width(Length::Fixed(180.0)),
                content
            ]
            .spacing(16)
            .width(Length::Fill)
            .height(Length::Fill)
        ]
        .spacing(12)
        .padding(16)
        .height(Length::Fill);

        if let Some(error_message) = self.error_message.as_ref() {
            root = root.push(text(error_message.clone()));
        } else if let Some(first_error) = validation_errors.first() {
            root = root.push(text(first_error.clone()));
        }

        let buttons = row![
            button(ui_text(language, UiText::Cancel)).on_press(Message::Cancel),
            button(ui_text(language, UiText::Ok))
                .on_press_maybe(can_submit.then_some(Message::Submit)),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        root.push(container(buttons).width(Length::Fill)).into()
    }

    fn page_content(&self) -> El<'_> {
        match self.page {
            SettingsPage::General => self.general_page(),
            SettingsPage::Input => self.input_page(),
            SettingsPage::Video => self.video_page(),
            SettingsPage::Audio => self.audio_page(),
            SettingsPage::System => self.system_page(),
        }
    }

    fn general_page(&self) -> El<'_> {
        let language = self.language();
        let mut content = column![
            labeled_pick_list(
                ui_text(language, UiText::Language),
                language_options(language),
                selected_choice(
                    self.draft.shared.general.language,
                    language_options(language)
                ),
                Message::SetLanguage
            ),
            labeled_pick_list(
                ui_text(language, UiText::SaveStoragePolicy),
                storage_policy_options(language),
                selected_choice(
                    self.draft.shared.persistence.storage_policy,
                    storage_policy_options(language)
                ),
                Message::SetStoragePolicy
            ),
        ]
        .spacing(16);

        if matches!(
            self.draft.shared.persistence.storage_policy,
            StoragePolicy::CustomDirectory
        ) {
            let storage_row = row![
                text(ui_text(language, UiText::SaveStorageDirectory)).width(Length::Fixed(220.0)),
                text_input("", &self.storage_directory_input)
                    .on_input(Message::SetStorageDirectory)
                    .width(Length::Fill),
                button(ui_text(language, UiText::Browse)).on_press(Message::BrowseStorageDirectory),
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            content = content.push(storage_row);
            if let Some(error) = self.storage_error() {
                content = content.push(text(error));
            }
        }

        content.into()
    }

    fn input_page(&self) -> El<'_> {
        let language = self.language();
        let factories = self.registry.all();
        let Some(input_tab_index) = self.input_tab_index else {
            return column![text("No systems available").size(14)].into();
        };
        let factory = &factories[input_tab_index];

        let mut content = column![];

        let tab_row = row(factories.iter().enumerate().map(|(i, f)| {
            let btn_text = text(f.display_name()).size(14);
            if Some(i) == self.input_tab_index {
                button(btn_text).style(button::primary).into()
            } else {
                button(btn_text).on_press(Message::SelectInputTab(i)).into()
            }
        }))
        .spacing(4);
        content = content.push(tab_row);

        if let Some(conflict) = self.input_conflict() {
            content = content.push(text(conflict));
        }

        let input_factory = factory.input_system_factory();
        let (slots, controllers) = (input_factory.slots(), input_factory.controllers());
        if !controllers.is_empty() {
            // Build a set of occupied slot IDs
            let mut occupied = std::collections::HashSet::new();
            for (s, c_opt) in &self.controller_assignments {
                let profile = match c_opt {
                    Some(p) => p.as_ref(),
                    None => continue,
                };
                for ps in profile.port_sets() {
                    if ps.ports.contains(s) {
                        for &port in ps.ports {
                            occupied.insert(port);
                        }
                    }
                }
            }
            for slot in slots {
                // Build slot choices including a "None" option.
                let none = Choice {
                    value: String::new(),
                    label: ui_text(self.draft.shared.general.language, UiText::None).to_string(),
                };
                let mut slot_choices: Vec<Choice<String>> = vec![none];
                slot_choices.extend(
                    controllers
                        .iter()
                        .filter(|c| {
                            c.port_sets()
                                .iter()
                                .any(|ps| ps.ports.first() == Some(&slot.id))
                        })
                        .map(|c| Choice {
                            value: c.profile_id().to_string(),
                            label: c.label().to_string(),
                        }),
                );
                if occupied.contains(&slot.id)
                    && !self
                        .controller_assignments
                        .iter()
                        .any(|(s, c)| *s == slot.id && c.is_some())
                {
                    // Occupied by another slot's multi-port controller
                    content = content.push(text(format!("{} — (occupied)", slot.label)));
                    continue;
                }
                let current = self
                    .controller_assignments
                    .iter()
                    .find(|(s, _)| *s == slot.id)
                    .and_then(|(_, c)| c.as_ref())
                    .and_then(|id| {
                        slot_choices
                            .iter()
                            .find(|ch| ch.value == id.profile_id().as_str())
                            .cloned()
                    })
                    .or_else(|| slot_choices.first().cloned()); // default to "None"
                let pick = pick_list(slot_choices, current, move |choice: Choice<String>| {
                    let controller_id = if choice.value.is_empty() {
                        None
                    } else {
                        Some(choice.value)
                    };
                    Message::SetControllerSlot {
                        slot: slot.id,
                        controller_id,
                    }
                });
                content = content.push(text(slot.label)).push(pick);
            }
        }

        let sections = keyboard_binding_sections(&input_topology(self), factory.system_id());
        let mut navigation = row![].spacing(16).align_y(Alignment::Center);
        for (index, section) in sections.iter().enumerate() {
            navigation = navigation.push(input_section_radio_label(
                section.attachment_label,
                InputPageSection::Attachment(index),
                self.input_section,
            ));
        }
        navigation = navigation.push(input_section_radio(
            language,
            UiText::Shortcuts,
            InputPageSection::Shortcuts,
            self.input_section,
        ));

        let section = match self.input_section {
            InputPageSection::Attachment(index) => sections
                .get(index)
                .map(|section| {
                    let rows = section
                        .bindings
                        .clone()
                        .into_iter()
                        .map(|descriptor| {
                            (
                                descriptor.control_label,
                                CaptureTarget::Binding {
                                    system: descriptor.system,
                                    attachment: descriptor.attachment.as_str().to_string(),
                                    control: descriptor.control.as_str().to_string(),
                                },
                            )
                        })
                        .collect::<Vec<_>>();
                    self.input_section(section.attachment_label, rows.into_iter())
                })
                .unwrap_or_else(|| self.input_section("", std::iter::empty())),
            InputPageSection::Shortcuts => self.input_section(
                ui_text(language, UiText::Shortcuts),
                shortcut_descriptors().iter().map(|descriptor| {
                    (descriptor.label, CaptureTarget::Shortcut(descriptor.action))
                }),
            ),
        };

        content.push(navigation).push(section).spacing(16).into()
    }

    fn input_section<'a>(
        &'a self,
        title: &'static str,
        rows: impl Iterator<Item = (&'static str, CaptureTarget)> + 'a,
    ) -> El<'a> {
        let language = self.language();
        let current_capture = self.capture_target.lock().unwrap();
        let mut content = column![text(title)];
        for (label, target) in rows {
            let binding_label = if current_capture.as_ref() == Some(&target) {
                ui_text(language, UiText::CapturePrompt)
            } else {
                current_binding_label(&self.draft, &target)
                    .unwrap_or(ui_text(language, UiText::Unbound))
            };
            content = content.push(
                row![
                    text(label).width(Length::Fixed(180.0)),
                    text(binding_label).width(Length::Fill),
                    button(ui_text(language, UiText::Change))
                        .on_press(Message::StartCapture(target.clone())),
                    button(ui_text(language, UiText::Clear))
                        .on_press(Message::ClearCapture(target)),
                ]
                .spacing(12)
                .width(Length::Fill)
                .align_y(Alignment::Center),
            );
        }
        content.spacing(8).into()
    }

    fn video_page(&self) -> El<'_> {
        let language = self.language();
        column![
            checkbox(self.draft.local.video.window.fullscreen_default)
                .label(ui_text(language, UiText::FullscreenDefault))
                .on_toggle(Message::ToggleFullscreenDefault),
            labeled_pick_list(
                ui_text(language, UiText::Scaling),
                scaling_options(language),
                selected_choice(
                    self.draft.local.video.window.scaling,
                    scaling_options(language)
                ),
                Message::SetScaling
            ),
            checkbox(self.draft.local.video.presentation.vsync)
                .label(ui_text(language, UiText::Vsync))
                .on_toggle(Message::ToggleVsync),
        ]
        .spacing(16)
        .into()
    }

    fn audio_page(&self) -> El<'_> {
        let language = self.language();
        column![
            checkbox(self.draft.local.audio.muted)
                .label(ui_text(language, UiText::Mute))
                .on_toggle(Message::ToggleMute),
            labeled_slider(
                ui_text(language, UiText::MasterVolume),
                format!("{}%", self.draft.local.audio.master_volume_percent),
                slider(
                    0..=100,
                    self.draft.local.audio.master_volume_percent,
                    Message::SetVolume
                )
            ),
            labeled_pick_list(
                ui_text(language, UiText::SampleRate),
                sample_rate_options(&self.audio_registry),
                selected_choice(
                    self.draft.local.audio.sample_rate,
                    sample_rate_options(&self.audio_registry)
                ),
                Message::SetSampleRate
            ),
            labeled_slider(
                ui_text(language, UiText::AudioLatency),
                format!("{} ms", self.draft.local.audio.latency_ms),
                slider(
                    10..=200,
                    self.draft.local.audio.latency_ms,
                    Message::SetLatency
                )
            ),
        ]
        .spacing(16)
        .into()
    }

    fn system_page(&self) -> El<'_> {
        let language = self.draft.shared.general.language;
        let factories = self.registry.all();
        let Some(system_tab_index) = self.system_tab_index else {
            return column![text("No systems available").size(14)].into();
        };
        let factory = &factories[system_tab_index];
        let system_id = factory.system_id();
        let view = settings_view(&self.draft, &system_id);
        let model = factory.settings_page(&view);

        let mut content = column![];
        let tab_labels: Vec<_> = factories.iter().map(|f| f.display_name()).collect();
        let tab_row = row(tab_labels.iter().enumerate().map(|(i, name)| {
            let btn_text = text(*name).size(14);
            if Some(i) == self.system_tab_index {
                button(btn_text).style(button::primary).into()
            } else {
                button(btn_text)
                    .on_press(Message::SelectSystemTab(i))
                    .into()
            }
        }))
        .spacing(4);
        content = content.push(tab_row);

        for field in model.fields.iter() {
            content = content.push(system_choice_row(field, language));
        }
        content.spacing(16).into()
    }
}

// ---------------------------------------------------------------------------
// Helper functions (shared between old and new paths)
// ---------------------------------------------------------------------------

fn page_radio(
    language: AppLanguage,
    label: UiText,
    value: SettingsPage,
    selected: SettingsPage,
) -> El<'static> {
    radio(
        ui_text(language, label),
        value,
        Some(selected),
        Message::SelectPage,
    )
    .into()
}

fn input_section_radio(
    language: AppLanguage,
    label: UiText,
    value: InputPageSection,
    selected: InputPageSection,
) -> El<'static> {
    input_section_radio_label(ui_text(language, label), value, selected)
}

fn input_section_radio_label(
    label: &'static str,
    value: InputPageSection,
    selected: InputPageSection,
) -> El<'static> {
    radio(label, value, Some(selected), Message::SelectInputSection).into()
}

fn labeled_pick_list<T: Clone + Eq + 'static>(
    label: &'static str,
    options: impl Into<Vec<Choice<T>>>,
    selected: Choice<T>,
    on_select: fn(Choice<T>) -> Message,
) -> El<'static> {
    let options = options.into();
    row![
        text(label).width(Length::Fixed(220.0)),
        pick_list(options, Some(selected), on_select).width(Length::Shrink)
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

fn labeled_slider<'a>(label: &'static str, value: String, slider: impl Into<El<'a>>) -> El<'a> {
    row![
        text(label).width(Length::Fixed(220.0)),
        slider.into(),
        text(value).width(Length::Fixed(72.0)),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

fn selected_choice<T: Clone + Eq>(value: T, options: impl Into<Vec<Choice<T>>>) -> Choice<T> {
    options
        .into()
        .into_iter()
        .find(|choice| choice.value == value)
        .unwrap()
}

fn system_choice_row(
    field: &SystemSettingsFieldModel,
    language: nerust_gui_settings::language::AppLanguage,
) -> El<'static> {
    let SystemSettingsFieldKind::Choice { selected, options } = &field.kind;
    let choices = options
        .iter()
        .map(|option| Choice {
            value: option.id.as_str().to_string(),
            label: resolve_label(option.label_id, language),
        })
        .collect::<Vec<_>>();
    let selected = choices
        .iter()
        .find(|choice| choice.value == selected.as_str())
        .cloned()
        .or_else(|| choices.first().cloned())
        .unwrap_or(Choice {
            value: String::new(),
            label: String::new(),
        });
    let field_id = field.id.as_str().to_string();
    row![
        text(resolve_label(field.label_id, language)).width(Length::Fixed(220.0)),
        pick_list(choices, Some(selected), move |choice| {
            Message::SetSystemChoice(field_id.clone(), choice)
        })
        .width(Length::Shrink)
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

#[cfg(target_os = "windows")]
fn default_font() -> Font {
    Font::with_name("Yu Gothic UI")
}

#[cfg(not(target_os = "windows"))]
fn default_font() -> Font {
    Font::DEFAULT
}

fn language_options(language: AppLanguage) -> Vec<Choice<AppLanguage>> {
    vec![
        Choice {
            value: AppLanguage::SystemDefault,
            label: ui_text(language, UiText::SystemDefault).to_string(),
        },
        Choice {
            value: AppLanguage::Japanese,
            label: ui_text(language, UiText::Japanese).to_string(),
        },
        Choice {
            value: AppLanguage::English,
            label: ui_text(language, UiText::English).to_string(),
        },
    ]
}

fn storage_policy_options(language: AppLanguage) -> Vec<Choice<StoragePolicy>> {
    vec![
        Choice {
            value: StoragePolicy::Sidecar,
            label: ui_text(language, UiText::Sidecar).to_string(),
        },
        Choice {
            value: StoragePolicy::AppSharedData,
            label: ui_text(language, UiText::AppSharedData).to_string(),
        },
        Choice {
            value: StoragePolicy::CustomDirectory,
            label: ui_text(language, UiText::CustomDirectory).to_string(),
        },
    ]
}

fn scaling_options(language: AppLanguage) -> Vec<Choice<ScalingMode>> {
    vec![
        Choice {
            value: ScalingMode::FitToWindow,
            label: ui_text(language, UiText::FitToWindow).to_string(),
        },
        Choice {
            value: ScalingMode::X1,
            label: "1x".to_string(),
        },
        Choice {
            value: ScalingMode::X2,
            label: "2x".to_string(),
        },
        Choice {
            value: ScalingMode::X3,
            label: "3x".to_string(),
        },
        Choice {
            value: ScalingMode::X4,
            label: "4x".to_string(),
        },
        Choice {
            value: ScalingMode::X5,
            label: "5x".to_string(),
        },
    ]
}

const FALLBACK_SAMPLE_RATES: [u32; 2] = [44_100, 48_000];

fn sample_rate_options(registry: &AudioBackendRegistry) -> Vec<Choice<u32>> {
    let rates = registry.supported_rates();
    let rates = if rates.is_empty() {
        &FALLBACK_SAMPLE_RATES
    } else {
        rates
    };
    rates
        .iter()
        .map(|&r| Choice {
            value: r,
            label: format!("{r}"),
        })
        .collect()
}

fn input_topology(state: &SettingsAppState) -> InputTopologyDescriptor {
    use nerust_gui_shell::session::input::build_topology;
    let Some(factory) = state
        .input_tab_index
        .and_then(|i| state.registry.all().get(i))
    else {
        return InputTopologyDescriptor {
            ports: Vec::new(),
            devices: Vec::new(),
        };
    };
    let slots = factory.input_system_factory().slots();
    build_topology(&state.controller_assignments, slots)
}

pub(crate) fn keyboard_key_from_physical(physical: iced::keyboard::key::Physical) -> Option<Key> {
    physical.try_into().ok()
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key::{Code, Physical};
    use nerust_keyboard::Key;

    use super::keyboard_key_from_physical;

    #[test]
    fn physical_key_mapping_matches_tao_bindings() {
        assert_eq!(
            keyboard_key_from_physical(Physical::Code(Code::KeyZ)),
            Some(Key::KeyZ)
        );
        assert_eq!(
            keyboard_key_from_physical(Physical::Code(Code::ArrowLeft)),
            Some(Key::ArrowLeft)
        );
        assert_eq!(
            keyboard_key_from_physical(Physical::Code(Code::F11)),
            Some(Key::F11)
        );
    }
}
