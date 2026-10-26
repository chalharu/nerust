use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

use gio::glib::{
    self,
    object::{Cast as _, IsA},
};
use gtk::prelude::{
    BoxExt as _, ButtonExt as _, CheckButtonExt as _, ComboBoxExt as _, ComboBoxExtManual as _,
    DialogExt as _, EditableExt as _, GtkWindowExt as _, WidgetExt as _,
};
use nerust_frontend_host::settings::{SettingsController, SettingsEvent, find_duplicate_system_id};
use nerust_gui_shell::session::access::SessionControl;
use nerust_gui_viewmodel::settings::{
    ReadOnlyObservableProperty, Subscription,
    dto::{
        AudioView, BindingRowView, BindingValueView, ChoiceView, ControllerSlotView, GeneralView,
        InputTabView, NotificationView, SystemTabView, VideoView, choice_position,
    },
};

/// Rebuild a combo from shared choices and select `selected`.
fn refill_choices<T: Clone + PartialEq + Eq>(
    combo: &gtk::ComboBoxText,
    choices: &[ChoiceView<T>],
    selected: &T,
) {
    combo.remove_all();
    for choice in choices {
        combo.append_text(&choice.label);
    }
    combo.set_active(choice_position(choices, selected));
}

/// Read back the selected choice. `None` (no selection) is a no-op for
/// the caller: with index selection there is no corrupt-id fallback.
fn active_choice_view<T: Clone + PartialEq + Eq>(
    combo: &gtk::ComboBoxText,
    choices: &[ChoiceView<T>],
) -> Option<ChoiceView<T>> {
    combo
        .active()
        .and_then(|index| choices.get(index as usize))
        .cloned()
}
use nerust_settings_core::model::{language::AppLanguage, local::ScalingMode};
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::{UiText, notification_group_label, text as ui_text},
};

use crate::State;

// ---------------------------------------------------------------------------
// Widget group structs
// ---------------------------------------------------------------------------

struct GeneralWidgets {
    language_combo: gtk::ComboBoxText,
    storage_policy_combo: gtk::ComboBoxText,
    storage_dir_entry: gtk::Entry,
    storage_dir_row: gtk::Box,
    storage_error_label: gtk::Label,
    language_label: gtk::Label,
    storage_policy_label: gtk::Label,
    storage_dir_label: gtk::Label,
}

struct VideoWidgets {
    fullscreen_check: gtk::CheckButton,
    scaling_combo: gtk::ComboBoxText,
    scaling_label: gtk::Label,
    vsync_check: gtk::CheckButton,
}

struct AudioWidgets {
    mute_check: gtk::CheckButton,
    volume_spin: gtk::SpinButton,
    volume_label: gtk::Label,
    sample_rate_combo: gtk::ComboBoxText,
    sample_rate_label: gtk::Label,
    latency_spin: gtk::SpinButton,
    latency_label: gtk::Label,
}

struct NotificationWidgets {
    info_check: gtk::CheckButton,
    info_duration_spin: gtk::SpinButton,
    info_color_combo: gtk::ComboBoxText,
    warning_check: gtk::CheckButton,
    warning_duration_spin: gtk::SpinButton,
    warning_color_combo: gtk::ComboBoxText,
    error_check: gtk::CheckButton,
    error_modal_radio: gtk::CheckButton,
    error_onscreen_radio: gtk::CheckButton,
    error_color_combo: gtk::ComboBoxText,
    status_check: gtk::CheckButton,
    position_combo: gtk::ComboBoxText,
    status_position_combo: gtk::ComboBoxText,
    area_combo: gtk::ComboBoxText,
    status_area_combo: gtk::ComboBoxText,
}

struct InputTabWidgets {
    _notebook: gtk::Notebook,
    pages: Vec<gtk::Box>,
    section_notebooks: RefCell<Vec<Option<gtk::Notebook>>>,
    shortcuts_page: gtk::Box,
    shortcuts_tab_label: gtk::Label,
}

struct SystemTabWidgets {
    _notebook: gtk::Notebook,
    pages: Vec<gtk::Box>,
}

struct PreferencesWidgets {
    general: GeneralWidgets,
    video: VideoWidgets,
    audio: AudioWidgets,
    notifications: NotificationWidgets,
    input: InputTabWidgets,
    system: SystemTabWidgets,
    ok_button: gtk::Widget,
    cancel_button: gtk::Widget,
    error_label: gtk::Label,
    dialog: gtk::Dialog,
    stack: gtk::Stack,
    page_ids: Vec<&'static str>,
}

// ---------------------------------------------------------------------------
// PreferencesBinding
// ---------------------------------------------------------------------------

struct PreferencesBinding {
    self_weak: std::rc::Weak<PreferencesBinding>,
    /// Single settings state machine, shared with the tao twin: widgets
    /// emit `SettingsEvent`, the controller reduces. Page/tab position
    /// stays native (Stack/Notebook own it); the controller copy is unused.
    controller: RefCell<SettingsController>,
    _subscriptions: RefCell<Vec<Subscription>>,
    general: GeneralWidgets,
    video: VideoWidgets,
    audio: AudioWidgets,
    notifications: NotificationWidgets,
    input: InputTabWidgets,
    system: SystemTabWidgets,
    ok_button: gtk::Widget,
    cancel_button: gtk::Widget,
    error_label: gtk::Label,
    dialog: gtk::Dialog,
    stack: gtk::Stack,
    page_ids: Vec<&'static str>,
    refreshing: Cell<bool>,
}

impl PreferencesBinding {
    fn new(controller: SettingsController, widgets: PreferencesWidgets) -> Rc<Self> {
        let PreferencesWidgets {
            general,
            video,
            audio,
            notifications,
            input,
            system,
            ok_button,
            cancel_button,
            error_label,
            dialog,
            stack,
            page_ids,
        } = widgets;
        let gv = controller.vm.general.view.get();
        let vv = controller.vm.video.view.get();
        let av = controller.vm.audio.view.get();
        let nv = controller.vm.notifications.view.get();

        let binding: Rc<Self> = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| {
            // One observer per view (vec! literal, not a push chain).
            // Fixed views carry their value in the callback; the three
            // observers below read back state, so they defer a frame:
            // they also fire synchronously inside `update`, when the
            // controller is already mutably borrowed.
            let mut subs = vec![
                Self::observe_view(weak, &controller.vm.general.view, Self::refresh_general),
                Self::observe_view(weak, &controller.vm.video.view, Self::refresh_video),
                Self::observe_view(weak, &controller.vm.audio.view, Self::refresh_audio),
                Self::observe_view(
                    weak,
                    &controller.vm.notifications.view,
                    Self::refresh_notifications,
                ),
            ];
            for (index, input_vm) in controller.vm.inputs().iter().enumerate() {
                subs.push(input_vm.view.observe({
                    let weak = weak.clone();
                    move |v| {
                        let Some(b) = weak.upgrade() else { return };
                        b.with_refreshing(|| b.rebuild_input_page(index, v));
                    }
                }));
            }
            // The standalone shortcuts tab has its own projection, so it
            // subscribes once instead of refreshing with every system.
            subs.push(controller.vm.shortcuts.view.observe({
                let weak = weak.clone();
                move |_| {
                    let Some(b) = weak.upgrade() else { return };
                    schedule_idle(&b.self_weak, move |b| {
                        b.with_refreshing(|| b.rebuild_shortcuts_page());
                    });
                }
            }));
            for (index, system_vm) in controller.vm.systems().iter().enumerate() {
                subs.push(system_vm.view.observe({
                    let weak = weak.clone();
                    move |v| {
                        let Some(b) = weak.upgrade() else { return };
                        b.with_refreshing(|| b.rebuild_system_page(index, v));
                    }
                }));
            }
            subs.push(controller.vm.capture.view.observe({
                let weak = weak.clone();
                move |_| {
                    let Some(b) = weak.upgrade() else { return };
                    schedule_idle(&b.self_weak, move |b| {
                        b.with_refreshing(|| b.rebuild_all_input_pages());
                    });
                }
            }));
            subs.push(controller.vm.revision.observe({
                let weak = weak.clone();
                move |_| {
                    let Some(b) = weak.upgrade() else { return };
                    schedule_idle(&b.self_weak, move |b| b.refresh_validation());
                }
            }));

            Self {
                self_weak: weak.clone(),
                controller: RefCell::new(controller),
                _subscriptions: RefCell::new(subs),
                general,
                video,
                audio,
                notifications,
                input,
                system,
                ok_button,
                cancel_button,
                error_label,
                dialog,
                stack,
                page_ids,
                refreshing: Cell::new(false),
            }
        });

        binding.with_refreshing(|| {
            binding.refresh_general(&gv);
            binding.refresh_video(&vv);
            binding.refresh_audio(&av);
            binding.refresh_notifications(&nv);
            binding.rebuild_all_input_pages();
            binding.rebuild_all_system_pages();
        });
        binding.refresh_validation();

        binding
    }

    /// Observe one settings view, refreshing while guarded. Shared by
    /// the four fixed views so the subscription list stays a literal.
    fn observe_view<T>(
        weak: &std::rc::Weak<Self>,
        view: &ReadOnlyObservableProperty<T>,
        refresh: fn(&Self, &T),
    ) -> Subscription
    where
        T: Clone + PartialEq + 'static,
    {
        let weak = weak.clone();
        view.observe(move |v| {
            let Some(binding) = weak.upgrade() else {
                return;
            };
            binding.with_refreshing(|| refresh(&binding, v));
        })
    }

    fn with_refreshing(&self, f: impl FnOnce()) {
        let previous = self.refreshing.replace(true);
        f();
        self.refreshing.set(previous);
    }

    fn refresh_general(&self, view: &GeneralView) {
        // Update localized labels
        let lang = view.language;
        self.dialog
            .set_title(Some(ui_text(lang, UiText::Preferences)));
        for page_id in self.page_ids.iter() {
            if let Some(child) = self.stack.child_by_name(page_id) {
                let label = match *page_id {
                    "general" => ui_text(lang, UiText::General),
                    "input" => ui_text(lang, UiText::Input),
                    "video" => ui_text(lang, UiText::Video),
                    "audio" => ui_text(lang, UiText::Audio),
                    "notifications" => ui_text(lang, UiText::Notifications),
                    "system" => ui_text(lang, UiText::System),
                    _ => continue,
                };
                self.stack.page(&child).set_title(label);
            }
        }

        // Update button labels
        if let Some(btn) = self.ok_button.downcast_ref::<gtk::Button>() {
            btn.set_label(ui_text(lang, UiText::Ok));
        }
        if let Some(btn) = self.cancel_button.downcast_ref::<gtk::Button>() {
            btn.set_label(ui_text(lang, UiText::Cancel));
        }

        // Update row labels stored in widget groups
        self.general
            .language_label
            .set_text(ui_text(lang, UiText::Language));
        self.general
            .storage_policy_label
            .set_text(ui_text(lang, UiText::SaveStoragePolicy));
        self.general
            .storage_dir_label
            .set_text(ui_text(lang, UiText::SaveStorageDirectory));
        self.video
            .fullscreen_check
            .set_label(Some(ui_text(lang, UiText::FullscreenDefault)));
        self.video
            .scaling_label
            .set_text(ui_text(lang, UiText::Scaling));
        self.video
            .vsync_check
            .set_label(Some(ui_text(lang, UiText::Vsync)));
        self.audio
            .mute_check
            .set_label(Some(ui_text(lang, UiText::Mute)));
        self.audio
            .volume_label
            .set_text(ui_text(lang, UiText::MasterVolume));
        self.audio
            .sample_rate_label
            .set_text(ui_text(lang, UiText::SampleRate));
        self.audio
            .latency_label
            .set_text(ui_text(lang, UiText::AudioLatency));

        refill_choices(
            &self.general.language_combo,
            &view.language_choices,
            &view.language,
        );
        refill_choices(
            &self.general.storage_policy_combo,
            &view.storage_policy_choices,
            &view.storage_policy,
        );
        self.general
            .storage_dir_row
            .set_visible(view.show_storage_directory);
        if self.general.storage_dir_entry.text() != view.storage_directory {
            self.general
                .storage_dir_entry
                .set_text(&view.storage_directory);
        }
    }

    fn refresh_video(&self, view: &VideoView) {
        self.video
            .fullscreen_check
            .set_active(view.fullscreen_default);
        refill_choices(
            &self.video.scaling_combo,
            &view.scaling_choices,
            &view.scaling,
        );
        self.video.vsync_check.set_active(view.vsync);
    }

    fn refresh_notifications(&self, view: &NotificationView) {
        let n = &self.notifications;
        n.info_check.set_active(view.info_enabled);
        n.info_duration_spin
            .set_value(f64::from(view.info_duration_ms));
        refill_choices(&n.info_color_combo, &view.color_choices, &view.info_color);
        n.warning_check.set_active(view.warning_enabled);
        n.warning_duration_spin
            .set_value(f64::from(view.warning_duration_ms));
        refill_choices(
            &n.warning_color_combo,
            &view.color_choices,
            &view.warning_color,
        );
        n.error_check.set_active(view.error_enabled);
        n.error_modal_radio.set_active(view.error_modal);
        n.error_onscreen_radio.set_active(!view.error_modal);
        refill_choices(&n.error_color_combo, &view.color_choices, &view.error_color);
        // Color only affects the on-screen pill; a modal dialog has
        // no tinted text.
        n.error_color_combo.set_sensitive(!view.error_modal);
        n.status_check.set_active(view.status_enabled);
        refill_choices(&n.position_combo, &view.position_choices, &view.position);
        refill_choices(
            &n.status_position_combo,
            &view.position_choices,
            &view.status_position,
        );
        refill_choices(&n.area_combo, &view.area_mode_choices, &view.area_mode);
        refill_choices(
            &n.status_area_combo,
            &view.area_mode_choices,
            &view.status_area_mode,
        );
        // Same band: the transient area mode governs the edge, so the
        // status picker is insensitive (an overlay status would dangle
        // half inside the reserved strip).
        n.status_area_combo
            .set_sensitive(!view.position.same_edge(view.status_position));
    }

    fn refresh_audio(&self, view: &AudioView) {
        self.audio.mute_check.set_active(view.muted);
        self.audio
            .volume_spin
            .set_value(f64::from(view.volume_percent));
        refill_choices(
            &self.audio.sample_rate_combo,
            &view.sample_rate_choices,
            &view.sample_rate,
        );
        self.audio
            .latency_spin
            .set_value(f64::from(view.latency_ms));
    }

    fn refresh_validation(&self) {
        // Shared banner policy, owned by the host controller.
        let banner = self.controller.borrow().validation_banner();
        self.ok_button.set_sensitive(banner.can_submit);
        self.error_label
            .set_text(banner.headline.as_deref().unwrap_or(""));
        self.general
            .storage_error_label
            .set_text(banner.storage.as_deref().unwrap_or(""));
    }

    fn rebuild_all_system_pages(&self) {
        for (index, vm) in self.controller.borrow().vm.systems().iter().enumerate() {
            self.rebuild_system_page(index, &vm.view.get());
        }
    }

    fn rebuild_system_page(&self, index: usize, view: &SystemTabView) {
        let Some(page) = self.system.pages.get(index) else {
            return;
        };
        clear_box(page);
        for field in &view.fields {
            let combo = gtk::ComboBoxText::new();
            for choice in &field.choices {
                combo.append(Some(choice.value.as_str()), &choice.label);
            }
            combo.set_active_id(Some(field.selected.as_str()));
            let field_id = field.id.clone();
            let choices = field.choices.clone();
            let weak = self.self_weak.clone();
            combo.connect_changed(move |combo| {
                let Some(binding) = weak.upgrade() else {
                    return;
                };
                if binding.refreshing.get() {
                    return;
                }
                let Some(active) = combo.active_id() else {
                    return;
                };
                let Some(choice) = choices
                    .iter()
                    .find(|choice| choice.value.as_str() == active)
                else {
                    return;
                };
                send(&binding, SettingsEvent::SelectSystemTab(index));
                send(
                    &binding,
                    SettingsEvent::SetSystemChoice(field_id.as_str().to_string(), choice.clone()),
                );
                if !binding
                    .controller
                    .borrow()
                    .error_message
                    .clone()
                    .unwrap_or_default()
                    .is_empty()
                {
                    let idx = index;
                    schedule_idle(&binding.self_weak, move |b| {
                        b.with_refreshing(|| {
                            if let Some(vm) = b.controller.borrow().vm.systems().get(idx) {
                                b.rebuild_system_page(idx, &vm.view.get());
                            }
                        });
                    });
                }
            });
            page.append(&labeled_row(&field.label, &combo));
        }
    }

    fn rebuild_all_input_pages(&self) {
        for (index, vm) in self.controller.borrow().vm.inputs().iter().enumerate() {
            self.rebuild_input_page(index, &vm.view.get());
        }
        self.rebuild_shortcuts_page();
    }

    /// Global shortcuts tab: standalone content outside every machine
    /// page, sourced from the dedicated shortcuts projection.
    fn rebuild_shortcuts_page(&self) {
        let page = &self.input.shortcuts_page;
        clear_box(page);
        self.input
            .shortcuts_tab_label
            .set_text(ui_text(self.language(), UiText::Shortcuts));
        let view = self.controller.borrow().vm.shortcuts.view.get();
        let shortcuts_label = gtk::Label::new(Some(&view.label));
        shortcuts_label.set_xalign(0.0);
        page.append(&shortcuts_label);
        let shortcuts_page = input_section_page();
        for row in &view.rows {
            self.append_binding_row(&shortcuts_page, row);
        }
        page.append(&shortcuts_page);
    }

    fn build_section_notebook(&self, view: &InputTabView) -> gtk::Notebook {
        let sections_notebook = gtk::Notebook::new();
        sections_notebook.set_scrollable(true);
        sections_notebook.set_tab_pos(gtk::PositionType::Top);
        sections_notebook.set_hexpand(true);
        sections_notebook.set_vexpand(true);
        for section in &view.sections {
            let section_page = input_section_page();
            for row in &section.rows {
                self.append_binding_row(&section_page, row);
            }
            sections_notebook
                .append_page(&section_page, Some(&gtk::Label::new(Some(&section.label))));
        }
        sections_notebook
    }

    fn rebuild_input_page(&self, index: usize, view: &InputTabView) {
        let Some(page) = self.input.pages.get(index) else {
            return;
        };
        let selected_section = self
            .input
            .section_notebooks
            .borrow()
            .get(index)
            .and_then(|notebook| notebook.as_ref())
            .and_then(gtk::Notebook::current_page);
        clear_box(page);
        add_conflict_labels(view, page);
        for slot in &view.slots {
            let combo = connect_slot_combo(slot, self.language(), index, &self.self_weak);
            page.append(&labeled_row(&slot.label, &combo));
        }

        let sections_notebook = self.build_section_notebook(view);
        if let Some(selected) = selected_section
            && selected < sections_notebook.n_pages()
        {
            sections_notebook.set_current_page(Some(selected));
        }
        page.append(&sections_notebook);
        if let Some(notebook) = self.input.section_notebooks.borrow_mut().get_mut(index) {
            *notebook = Some(sections_notebook);
        }
    }

    fn append_binding_row(&self, page: &gtk::Box, row: &BindingRowView) {
        let value = match &row.value {
            BindingValueView::Unbound(value)
            | BindingValueView::Bound(value)
            | BindingValueView::Capturing(value) => value,
        };
        self.append_capture_row(page, &row.label, value, row.target.clone());
    }

    fn append_capture_row(&self, page: &gtk::Box, label: &str, value: &str, target: CaptureTarget) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = gtk::Label::new(Some(label));
        name.set_xalign(0.0);
        name.set_width_chars(20);
        let current = gtk::Label::new(Some(value));
        current.set_xalign(0.0);
        current.set_hexpand(true);
        let change = gtk::Button::with_label(ui_text(self.language(), UiText::Change));
        let clear = gtk::Button::with_label(ui_text(self.language(), UiText::Clear));
        let weak_c = self.self_weak.clone();
        let change_target_c = target.clone();
        change.connect_clicked(move |_| {
            if let Some(binding) = weak_c.upgrade() {
                send(
                    &binding,
                    SettingsEvent::StartCapture(change_target_c.clone()),
                );
            }
        });
        let weak_cl = self.self_weak.clone();
        clear.connect_clicked(move |_| {
            if let Some(binding) = weak_cl.upgrade() {
                send(&binding, SettingsEvent::ClearCapture(target.clone()));
            }
        });
        row.append(&name);
        row.append(&current);
        row.append(&change);
        row.append(&clear);
        page.append(&row);
    }

    fn language(&self) -> AppLanguage {
        self.controller.borrow().vm.general.view.get().language
    }
}

// ---------------------------------------------------------------------------
// Main dialog entry point
// ---------------------------------------------------------------------------

pub(crate) fn present_preferences_dialog(
    parent: &gtk::ApplicationWindow,
    state: Rc<RefCell<State>>,
    on_close: impl FnOnce() + 'static,
) -> Option<gtk::Dialog> {
    let (snapshot, registry, audio_registry) = {
        let s = state.borrow();
        (
            s.settings_snapshot().clone(),
            s.ctx.registry.clone(),
            s.ctx.audio_registry.clone(),
        )
    };
    let lang = snapshot.shared.general.language;

    // Same duplicate-SystemId guard as the tao twin: refuse the window
    // rather than failing the view model mid-construction.
    if let Some(duplicate) = find_duplicate_system_id(&registry) {
        let mut state = state.borrow_mut();
        state.host.session_mut().report_external_error(
            nerust_gui_shell::session::notification::FailureOrigin::Host,
            format!("duplicate SystemId '{duplicate}'"),
        );
        drop(state);
        on_close();
        return None;
    }

    // Factory dedup, sample-rate fallback, and the FS path validator all
    // live in the shared controller; the dialog only owns widgets.
    let controller = SettingsController::new(
        &snapshot,
        Arc::clone(&registry),
        Arc::clone(&audio_registry),
    );

    let dialog = gtk::Dialog::builder()
        .transient_for(parent)
        .modal(true)
        .destroy_with_parent(true)
        .title(ui_text(lang, UiText::Preferences))
        .default_width(900)
        .default_height(560)
        .build();
    let cancel_button = dialog.add_button(ui_text(lang, UiText::Cancel), gtk::ResponseType::Cancel);
    let ok_button = dialog.add_button(ui_text(lang, UiText::Ok), gtk::ResponseType::Ok);
    if let Some(action_box) = ok_button
        .parent()
        .and_then(|parent| parent.downcast::<gtk::Box>().ok())
    {
        action_box.set_spacing(12);
        action_box.set_margin_top(12);
        action_box.set_margin_bottom(12);
        action_box.set_margin_start(12);
        action_box.set_margin_end(12);
    }

    let content = dialog.content_area();
    content.set_spacing(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_vexpand(true);

    let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    root.set_hexpand(true);
    root.set_vexpand(true);
    content.append(&root);

    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    let sidebar = gtk::StackSidebar::new();
    sidebar.set_stack(&stack);
    sidebar.set_vexpand(true);
    root.append(&sidebar);
    root.append(&stack);

    let (general_page_scroller, general_page) = stack_page();
    let (input_page_scroller, input_page) = stack_page();
    let (video_page_scroller, video_page) = stack_page();
    let (audio_page_scroller, audio_page) = stack_page();
    let (notifications_page_scroller, notifications_page) = stack_page();
    let (system_page_scroller, system_page) = stack_page();
    stack.add_titled(
        &general_page_scroller,
        Some("general"),
        ui_text(lang, UiText::General),
    );
    stack.add_titled(
        &input_page_scroller,
        Some("input"),
        ui_text(lang, UiText::Input),
    );
    stack.add_titled(
        &video_page_scroller,
        Some("video"),
        ui_text(lang, UiText::Video),
    );
    stack.add_titled(
        &audio_page_scroller,
        Some("audio"),
        ui_text(lang, UiText::Audio),
    );
    stack.add_titled(
        &notifications_page_scroller,
        Some("notifications"),
        ui_text(lang, UiText::Notifications),
    );
    stack.add_titled(
        &system_page_scroller,
        Some("system"),
        ui_text(lang, UiText::System),
    );

    let error_label = gtk::Label::new(None);
    error_label.set_xalign(0.0);
    content.append(&error_label);

    // ---- General page ----
    let language_label = gtk::Label::new(Some("Language"));
    let language_combo = combo_box(&[
        ("system_default", ui_text(lang, UiText::SystemDefault)),
        ("japanese", ui_text(lang, UiText::Japanese)),
        ("english", ui_text(lang, UiText::English)),
    ]);
    let lang_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    lang_row.append(&language_label);
    lang_row.append(&language_combo);
    general_page.append(&lang_row);

    let storage_policy_label = gtk::Label::new(Some(ui_text(lang, UiText::SaveStoragePolicy)));
    let storage_policy_combo = combo_box(&[
        ("sidecar", ui_text(lang, UiText::Sidecar)),
        ("app_shared_data", ui_text(lang, UiText::AppSharedData)),
        ("custom_directory", ui_text(lang, UiText::CustomDirectory)),
    ]);
    let sp_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    sp_row.append(&storage_policy_label);
    sp_row.append(&storage_policy_combo);
    general_page.append(&sp_row);

    let storage_dir_label = gtk::Label::new(Some(ui_text(lang, UiText::SaveStorageDirectory)));
    let storage_dir_entry = gtk::Entry::new();
    let storage_dir_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    storage_dir_row.append(&storage_dir_label);
    storage_dir_row.append(&storage_dir_entry);
    let storage_error_label = gtk::Label::new(None);
    storage_error_label.set_xalign(0.0);
    general_page.append(&storage_dir_row);
    general_page.append(&storage_error_label);

    // ---- Video page ----
    let fullscreen_check = gtk::CheckButton::with_label(ui_text(lang, UiText::FullscreenDefault));
    video_page.append(&fullscreen_check);
    let scaling_label = gtk::Label::new(Some(ui_text(lang, UiText::Scaling)));
    let scaling_combo = combo_box(&[
        ("fit", ui_text(lang, UiText::FitToWindow)),
        ("1", "1x"),
        ("2", "2x"),
        ("3", "3x"),
        ("4", "4x"),
        ("5", "5x"),
    ]);
    let scaling_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    scaling_row.append(&scaling_label);
    scaling_row.append(&scaling_combo);
    video_page.append(&scaling_row);
    let vsync_check = gtk::CheckButton::with_label(ui_text(lang, UiText::Vsync));
    video_page.append(&vsync_check);

    // ---- Audio page ----
    let mute_check = gtk::CheckButton::with_label(ui_text(lang, UiText::Mute));
    audio_page.append(&mute_check);
    let volume_label = gtk::Label::new(Some(ui_text(lang, UiText::MasterVolume)));
    let volume_spin = gtk::SpinButton::with_range(0.0, 100.0, 1.0);
    let vol_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    vol_row.append(&volume_label);
    vol_row.append(&volume_spin);
    audio_page.append(&vol_row);
    let sample_rate_label = gtk::Label::new(Some(ui_text(lang, UiText::SampleRate)));
    let sample_rate_combo = {
        // Initial fill only; refresh rebuilds from the shared choices.
        let combo = gtk::ComboBoxText::new();
        for choice in &controller.vm.audio.view.get().sample_rate_choices {
            combo.append_text(&choice.label);
        }
        combo
    };
    let sr_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    sr_row.append(&sample_rate_label);
    sr_row.append(&sample_rate_combo);
    audio_page.append(&sr_row);
    let latency_label = gtk::Label::new(Some(ui_text(lang, UiText::AudioLatency)));
    let latency_spin = gtk::SpinButton::with_range(10.0, 200.0, 1.0);
    let lat_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    lat_row.append(&latency_label);
    lat_row.append(&latency_spin);
    audio_page.append(&lat_row);

    // ---- Notifications page: one section per level plus
    // display/status groups, so each control's scope reads off its
    // header. Error presentation is an exclusive radio pair.
    let info_check = gtk::CheckButton::with_label(ui_text(lang, UiText::NotificationEnabled));
    section_header_toggled(
        &notifications_page,
        ui_text(lang, UiText::NotificationInfo),
        &info_check,
        &notification_group_label(lang, ui_text(lang, UiText::NotificationInfo)),
        true,
    );
    let info_duration_spin = gtk::SpinButton::with_range(1000.0, 30000.0, 500.0);
    notifications_page.append(&labeled_sub_row(
        ui_text(lang, UiText::NotificationDuration),
        &info_duration_spin,
    ));
    let info_color_combo = combo_box(&[
        ("white", ui_text(lang, UiText::ColorWhite)),
        ("yellow", ui_text(lang, UiText::ColorYellow)),
        ("red", ui_text(lang, UiText::ColorRed)),
        ("green", ui_text(lang, UiText::ColorGreen)),
        ("cyan", ui_text(lang, UiText::ColorCyan)),
    ]);
    notifications_page.append(&labeled_sub_row(
        ui_text(lang, UiText::NotificationColor),
        &info_color_combo,
    ));
    let warning_check = gtk::CheckButton::with_label(ui_text(lang, UiText::NotificationEnabled));
    section_header_toggled(
        &notifications_page,
        ui_text(lang, UiText::NotificationWarning),
        &warning_check,
        &notification_group_label(lang, ui_text(lang, UiText::NotificationWarning)),
        false,
    );
    let warning_duration_spin = gtk::SpinButton::with_range(1000.0, 30000.0, 500.0);
    notifications_page.append(&labeled_sub_row(
        ui_text(lang, UiText::NotificationDuration),
        &warning_duration_spin,
    ));
    let warning_color_combo = combo_box(&[
        ("white", ui_text(lang, UiText::ColorWhite)),
        ("yellow", ui_text(lang, UiText::ColorYellow)),
        ("red", ui_text(lang, UiText::ColorRed)),
        ("green", ui_text(lang, UiText::ColorGreen)),
        ("cyan", ui_text(lang, UiText::ColorCyan)),
    ]);
    notifications_page.append(&labeled_sub_row(
        ui_text(lang, UiText::NotificationColor),
        &warning_color_combo,
    ));
    let error_check = gtk::CheckButton::with_label(ui_text(lang, UiText::NotificationEnabled));
    section_header_toggled(
        &notifications_page,
        ui_text(lang, UiText::Error),
        &error_check,
        &notification_group_label(lang, ui_text(lang, UiText::Error)),
        false,
    );
    let error_modal_radio =
        gtk::CheckButton::with_label(ui_text(lang, UiText::NotificationErrorModal));
    let error_onscreen_radio =
        gtk::CheckButton::with_label(ui_text(lang, UiText::NotificationErrorOnScreen));
    error_onscreen_radio.set_group(Some(&error_modal_radio));
    error_modal_radio.set_margin_start(16);
    error_onscreen_radio.set_margin_start(16);
    notifications_page.append(&error_modal_radio);
    notifications_page.append(&error_onscreen_radio);
    let error_color_combo = combo_box(&[
        ("white", ui_text(lang, UiText::ColorWhite)),
        ("yellow", ui_text(lang, UiText::ColorYellow)),
        ("red", ui_text(lang, UiText::ColorRed)),
        ("green", ui_text(lang, UiText::ColorGreen)),
        ("cyan", ui_text(lang, UiText::ColorCyan)),
    ]);
    notifications_page.append(&labeled_sub_row(
        ui_text(lang, UiText::NotificationColor),
        &error_color_combo,
    ));
    section_header(
        &notifications_page,
        ui_text(lang, UiText::NotificationSectionDisplay),
    );
    let position_combo = combo_box(&[
        ("top_left", ui_text(lang, UiText::PosTopLeft)),
        ("top_right", ui_text(lang, UiText::PosTopRight)),
        ("bottom_left", ui_text(lang, UiText::PosBottomLeft)),
        ("bottom_right", ui_text(lang, UiText::PosBottomRight)),
    ]);
    notifications_page.append(&labeled_sub_row(
        ui_text(lang, UiText::NotificationPosition),
        &position_combo,
    ));
    let status_position_combo = combo_box(&[
        ("top_left", ui_text(lang, UiText::PosTopLeft)),
        ("top_right", ui_text(lang, UiText::PosTopRight)),
        ("bottom_left", ui_text(lang, UiText::PosBottomLeft)),
        ("bottom_right", ui_text(lang, UiText::PosBottomRight)),
    ]);
    let area_combo = combo_box(&[
        ("overlay", ui_text(lang, UiText::AreaOverlay)),
        ("strip", ui_text(lang, UiText::AreaStrip)),
    ]);
    notifications_page.append(&labeled_sub_row(
        ui_text(lang, UiText::NotificationAreaMode),
        &area_combo,
    ));
    let status_check = gtk::CheckButton::with_label(ui_text(lang, UiText::NotificationEnabled));
    section_header_toggled(
        &notifications_page,
        ui_text(lang, UiText::NotificationSectionStatus),
        &status_check,
        ui_text(lang, UiText::NotificationStatus),
        false,
    );
    notifications_page.append(&labeled_sub_row(
        ui_text(lang, UiText::NotificationStatusPosition),
        &status_position_combo,
    ));
    let status_area_combo = combo_box(&[
        ("overlay", ui_text(lang, UiText::AreaOverlay)),
        ("strip", ui_text(lang, UiText::AreaStrip)),
    ]);
    notifications_page.append(&labeled_sub_row(
        ui_text(lang, UiText::NotificationStatusAreaMode),
        &status_area_combo,
    ));

    // ---- Input page (tabs) ----
    let input_notebook = gtk::Notebook::new();
    input_notebook.set_scrollable(true);
    input_notebook.set_tab_pos(gtk::PositionType::Top);
    input_page.append(&input_notebook);
    let mut input_pages = Vec::new();
    for factory in registry.all() {
        let tab_label = gtk::Label::new(Some(factory.display_name()));
        let tab_page = gtk::Box::new(gtk::Orientation::Vertical, 6);
        tab_page.set_margin_start(6);
        tab_page.set_margin_end(6);
        tab_page.set_margin_top(6);
        tab_page.set_margin_bottom(6);
        input_notebook.append_page(&tab_page, Some(&tab_label));
        input_pages.push(tab_page);
    }
    // Trailing global-shortcuts tab: same row as the systems, never a
    // machine page.
    let shortcuts_tab_label = gtk::Label::new(Some(ui_text(lang, UiText::Shortcuts)));
    let shortcuts_tab_page = gtk::Box::new(gtk::Orientation::Vertical, 6);
    shortcuts_tab_page.set_margin_start(6);
    shortcuts_tab_page.set_margin_end(6);
    shortcuts_tab_page.set_margin_top(6);
    shortcuts_tab_page.set_margin_bottom(6);
    input_notebook.append_page(&shortcuts_tab_page, Some(&shortcuts_tab_label));

    // ---- System page (tabs) ----
    let system_notebook = gtk::Notebook::new();
    system_notebook.set_scrollable(true);
    system_notebook.set_tab_pos(gtk::PositionType::Top);
    system_page.append(&system_notebook);
    let mut system_pages = Vec::new();
    for factory in registry.all() {
        let tab_label = gtk::Label::new(Some(factory.display_name()));
        let tab_page = gtk::Box::new(gtk::Orientation::Vertical, 6);
        tab_page.set_margin_start(6);
        tab_page.set_margin_end(6);
        tab_page.set_margin_top(6);
        tab_page.set_margin_bottom(6);
        system_notebook.append_page(&tab_page, Some(&tab_label));
        system_pages.push(tab_page);
    }

    // Build binding
    let general_w = GeneralWidgets {
        language_combo: language_combo.clone(),
        storage_policy_combo: storage_policy_combo.clone(),
        storage_dir_entry: storage_dir_entry.clone(),
        storage_dir_row: storage_dir_row.clone(),
        storage_error_label: storage_error_label.clone(),
        language_label,
        storage_policy_label,
        storage_dir_label,
    };
    let video_w = VideoWidgets {
        fullscreen_check: fullscreen_check.clone(),
        scaling_combo: scaling_combo.clone(),
        scaling_label,
        vsync_check: vsync_check.clone(),
    };
    let audio_w = AudioWidgets {
        mute_check: mute_check.clone(),
        volume_spin: volume_spin.clone(),
        volume_label,
        sample_rate_combo: sample_rate_combo.clone(),
        sample_rate_label,
        latency_spin: latency_spin.clone(),
        latency_label,
    };
    let notifications_w = NotificationWidgets {
        info_check: info_check.clone(),
        info_duration_spin: info_duration_spin.clone(),
        info_color_combo: info_color_combo.clone(),
        warning_check: warning_check.clone(),
        warning_duration_spin: warning_duration_spin.clone(),
        warning_color_combo: warning_color_combo.clone(),
        error_check: error_check.clone(),
        error_modal_radio: error_modal_radio.clone(),
        error_onscreen_radio: error_onscreen_radio.clone(),
        error_color_combo: error_color_combo.clone(),
        status_check: status_check.clone(),
        position_combo: position_combo.clone(),
        status_position_combo: status_position_combo.clone(),
        area_combo: area_combo.clone(),
        status_area_combo: status_area_combo.clone(),
    };
    let input_w = InputTabWidgets {
        _notebook: input_notebook,
        section_notebooks: RefCell::new(vec![None; input_pages.len()]),
        pages: input_pages,
        shortcuts_page: shortcuts_tab_page,
        shortcuts_tab_label,
    };
    let system_w = SystemTabWidgets {
        _notebook: system_notebook,
        pages: system_pages,
    };

    let page_ids: Vec<&'static str> = vec![
        "general",
        "input",
        "video",
        "audio",
        "notifications",
        "system",
    ];
    let _binding = PreferencesBinding::new(
        controller,
        PreferencesWidgets {
            general: general_w,
            video: video_w,
            audio: audio_w,
            notifications: notifications_w,
            input: input_w,
            system: system_w,
            ok_button,
            cancel_button,
            error_label,
            dialog: dialog.clone(),
            stack: stack.clone(),
            page_ids,
        },
    );

    // ---- Signal handlers ----
    connect_choice(
        &language_combo,
        &weak_handler(&_binding),
        |b| b.controller.borrow().vm.general.view.get().language_choices,
        SettingsEvent::SetLanguage,
    );
    connect_choice(
        &storage_policy_combo,
        &weak_handler(&_binding),
        |b| {
            b.controller
                .borrow()
                .vm
                .general
                .view
                .get()
                .storage_policy_choices
        },
        SettingsEvent::SetStoragePolicy,
    );
    storage_dir_entry.connect_changed({
        let w = weak_handler(&_binding);
        move |entry| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() {
                return;
            }
            send(
                &b,
                SettingsEvent::SetStorageDirectory(entry.text().to_string()),
            );
        }
    });
    connect_toggle(
        &fullscreen_check,
        &weak_handler(&_binding),
        SettingsEvent::ToggleFullscreenDefault,
    );
    connect_choice(
        &scaling_combo,
        &weak_handler(&_binding),
        |b| b.controller.borrow().vm.video.view.get().scaling_choices,
        SettingsEvent::SetScaling,
    );
    connect_toggle(
        &vsync_check,
        &weak_handler(&_binding),
        SettingsEvent::ToggleVsync,
    );
    connect_toggle(
        &mute_check,
        &weak_handler(&_binding),
        SettingsEvent::ToggleMute,
    );
    connect_spin(&volume_spin, &weak_handler(&_binding), |v| {
        SettingsEvent::SetVolume(v as u8)
    });
    connect_choice(
        &sample_rate_combo,
        &weak_handler(&_binding),
        |b| {
            b.controller
                .borrow()
                .vm
                .audio
                .view
                .get()
                .sample_rate_choices
        },
        SettingsEvent::SetSampleRate,
    );
    connect_spin(&latency_spin, &weak_handler(&_binding), |v| {
        SettingsEvent::SetLatency(v as u16)
    });
    connect_toggle(
        &info_check,
        &weak_handler(&_binding),
        SettingsEvent::ToggleInfoEnabled,
    );
    connect_spin(&info_duration_spin, &weak_handler(&_binding), |v| {
        SettingsEvent::SetInfoDuration(v as u32)
    });
    connect_choice(
        &info_color_combo,
        &weak_handler(&_binding),
        |b| {
            b.controller
                .borrow()
                .vm
                .notifications
                .view
                .get()
                .color_choices
        },
        SettingsEvent::SetInfoColor,
    );
    connect_toggle(
        &warning_check,
        &weak_handler(&_binding),
        SettingsEvent::ToggleWarningEnabled,
    );
    connect_spin(&warning_duration_spin, &weak_handler(&_binding), |v| {
        SettingsEvent::SetWarningDuration(v as u32)
    });
    connect_choice(
        &warning_color_combo,
        &weak_handler(&_binding),
        |b| {
            b.controller
                .borrow()
                .vm
                .notifications
                .view
                .get()
                .color_choices
        },
        SettingsEvent::SetWarningColor,
    );
    connect_toggle(
        &error_check,
        &weak_handler(&_binding),
        SettingsEvent::ToggleErrorEnabled,
    );
    error_modal_radio.connect_toggled({
        let w = weak_handler(&_binding);
        move |button| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() || !button.is_active() {
                return;
            }
            send(&b, SettingsEvent::ToggleErrorModal(true));
        }
    });
    error_onscreen_radio.connect_toggled({
        let w = weak_handler(&_binding);
        move |button| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() || !button.is_active() {
                return;
            }
            send(&b, SettingsEvent::ToggleErrorModal(false));
        }
    });
    connect_choice(
        &error_color_combo,
        &weak_handler(&_binding),
        |b| {
            b.controller
                .borrow()
                .vm
                .notifications
                .view
                .get()
                .color_choices
        },
        SettingsEvent::SetErrorColor,
    );
    connect_toggle(
        &status_check,
        &weak_handler(&_binding),
        SettingsEvent::ToggleStatusEnabled,
    );
    connect_choice(
        &position_combo,
        &weak_handler(&_binding),
        |b| {
            b.controller
                .borrow()
                .vm
                .notifications
                .view
                .get()
                .position_choices
        },
        SettingsEvent::SetPosition,
    );
    connect_choice(
        &status_position_combo,
        &weak_handler(&_binding),
        |b| {
            b.controller
                .borrow()
                .vm
                .notifications
                .view
                .get()
                .position_choices
        },
        SettingsEvent::SetStatusPosition,
    );
    connect_choice(
        &area_combo,
        &weak_handler(&_binding),
        |b| {
            b.controller
                .borrow()
                .vm
                .notifications
                .view
                .get()
                .area_mode_choices
        },
        SettingsEvent::SetAreaMode,
    );
    connect_choice(
        &status_area_combo,
        &weak_handler(&_binding),
        |b| {
            b.controller
                .borrow()
                .vm
                .notifications
                .view
                .get()
                .area_mode_choices
        },
        SettingsEvent::SetStatusAreaMode,
    );

    let key_controller = gtk::EventControllerKey::new();
    key_controller.connect_key_pressed({
        let weak = Rc::downgrade(&_binding);
        move |_, key, _, _| {
            let Some(binding) = weak.upgrade() else {
                return gtk::glib::Propagation::Proceed;
            };
            if binding
                .controller
                .borrow()
                .vm
                .capture
                .view
                .get()
                .target
                .is_none()
            {
                return gtk::glib::Propagation::Proceed;
            }
            let Ok(mapped_key) = nerust_keyboard::Key::try_from(key) else {
                return gtk::glib::Propagation::Stop;
            };
            send(&binding, SettingsEvent::CaptureKey(mapped_key));
            gtk::glib::Propagation::Stop
        }
    });
    dialog.add_controller(key_controller);

    // Submit
    let parent_clone = parent.clone();
    let state_clone = state.clone();
    let finish_cb = Rc::new(RefCell::new(Some(Box::new(on_close) as Box<dyn FnOnce()>)));
    connect_preferences_finish(&dialog, &finish_cb);
    let _binding_owned = Rc::clone(&_binding);
    dialog.connect_response(move |dialog, response| {
        if response != gtk::ResponseType::Ok {
            dialog.destroy();
            finish_preferences(&finish_cb);
            return;
        }
        // The controller stages the snapshot in its mailbox on success;
        // validation failures leave the dialog open with the banner shown.
        _binding_owned
            .controller
            .borrow_mut()
            .update(SettingsEvent::Submit);
        let Some(snapshot) = _binding_owned.controller.borrow().take_pending_apply() else {
            return;
        };
        let scaling = snapshot.local.video.window.scaling;
        let fullscreen_default = snapshot.local.video.window.fullscreen_default;
        let apply_result = with_mut(&state_clone, |state| state.apply_settings(snapshot));
        match apply_result {
            Ok(result) => {
                if result.fullscreen_default_changed {
                    parent_clone.set_fullscreened(fullscreen_default);
                }
                if result.scaling_changed
                    && let Some(profile) = state_clone.borrow().render_profile()
                {
                    apply_scaling_to_window(&parent_clone, scaling, &profile);
                }
                dialog.destroy();
                finish_preferences(&finish_cb);
            }
            Err(e) => {
                _binding_owned.error_label.set_text(&e.to_string());
            }
        }
    });

    dialog.present();
    Some(dialog)
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

type FinishCallback = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

fn connect_preferences_finish(dialog: &gtk::Dialog, callback: &FinishCallback) {
    // Native destruction bypasses response. In GTK4, strong references can
    // delay disposal/destroy, so unrealize also releases the native lifetime.
    let on_destroy = callback.clone();
    dialog.connect_destroy(move |_| finish_preferences(&on_destroy));
    let on_unrealize = callback.clone();
    dialog.connect_unrealize(move |_| finish_preferences(&on_unrealize));
    let on_close = callback.clone();
    dialog.connect_close_request(move |_| {
        finish_preferences(&on_close);
        glib::Propagation::Proceed
    });
}

fn finish_preferences(callback: &FinishCallback) {
    // Release the RefCell borrow before invoking native/user callbacks, which
    // can synchronously trigger another close or destruction signal.
    let finish = callback.borrow_mut().take();
    if let Some(finish) = finish {
        finish();
    }
}

/// Titled settings group header: bold, larger than items, glued to
/// its group (2 below) with the gap above carried by the page.
/// Matches the tao form (16 bold vs 14 items; 24 between groups).
/// First group takes no top gap; the rest take 20.
fn section_header(container: &gtk::Box, title: &str) {
    section_header_spaced(container, title, 20);
}

/// Section header with the master toggle on the same row.
/// The generic "Enabled" carries the level as a tooltip.
fn section_header_toggled(
    container: &gtk::Box,
    title: &str,
    toggle: &gtk::CheckButton,
    accessible_label: &str,
    first: bool,
) {
    let label = gtk::Label::new(None);
    label.set_markup(&format!(
        "<b><span size=\"larger\">{}</span></b>",
        gtk::glib::markup_escape_text(title)
    ));
    label.set_xalign(0.0);
    // Governing toggle sits next to its title: Apple HIG aligns a
    // governing checkbox on the leading edge, and proximity beats
    // a full-width gap (Microsoft cards share one control column).
    toggle.set_tooltip_text(Some(accessible_label));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_margin_top(if first { 0 } else { 20 });
    row.set_margin_bottom(2);
    row.append(&label);
    row.append(toggle);
    container.append(&row);
}

fn section_header_spaced(container: &gtk::Box, title: &str, margin_top: i32) {
    let label = gtk::Label::new(None);
    label.set_markup(&format!(
        "<b><span size=\"larger\">{}</span></b>",
        gtk::glib::markup_escape_text(title)
    ));
    label.set_xalign(0.0);
    label.set_margin_top(margin_top);
    label.set_margin_bottom(2);
    container.append(&label);
}

fn labeled_row(label: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_width_chars(24);
    row.append(&label);
    row.append(widget);
    row
}

/// Subordinate row under a toggled section header: indented to show
/// the hierarchy (Apple HIG checkbox grouping).
fn labeled_sub_row(label: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = labeled_row(label, widget);
    row.set_margin_start(16);
    row
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn with_mut<T, R>(value: &RefCell<T>, f: impl FnOnce(&mut T) -> R) -> R {
    let mut value = value.borrow_mut();
    f(&mut value)
}

fn input_section_page() -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    page.set_margin_start(6);
    page.set_margin_end(6);
    page.set_margin_top(6);
    page.set_margin_bottom(6);
    page
}

fn apply_scaling_to_window(
    window: &gtk::ApplicationWindow,
    scaling: ScalingMode,
    render_profile: &nerust_render_traits::VideoRenderProfile,
) {
    let base_width = render_profile.physical_size.width as i32;
    let base_height = render_profile.physical_size.height as i32;
    let scale = match scaling {
        ScalingMode::FitToWindow => None,
        ScalingMode::X1 => Some(1),
        ScalingMode::X2 => Some(2),
        ScalingMode::X3 => Some(3),
        ScalingMode::X4 => Some(4),
        ScalingMode::X5 => Some(5),
    };
    if let Some(scale) = scale {
        window.set_default_size(base_width * scale, base_height * scale);
    }
    window.queue_resize();
}

fn stack_page() -> (gtk::ScrolledWindow, gtk::Box) {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_hexpand(true);

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_hexpand(true);
    scroller.set_vexpand(true);
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_propagate_natural_height(false);
    scroller.set_child(Some(&page));

    (scroller, page)
}

fn weak_handler(b: &Rc<PreferencesBinding>) -> std::rc::Weak<PreferencesBinding> {
    Rc::downgrade(b)
}

/// Execute `f` with an upgraded weak reference if the Rc is still alive.
/// Returns `true` if the weak was upgraded and `f` was called.
fn with_upgraded_weak<T, F>(weak: &std::rc::Weak<T>, f: F) -> bool
where
    F: FnOnce(&T),
{
    if let Some(b) = weak.upgrade() {
        f(&b);
        true
    } else {
        false
    }
}

/// Schedule a closure to run on the GLib main loop after the current
/// signal emission completes. The closure receives a strong reference
/// to the binding only if it is still alive.
fn schedule_idle<F>(weak: &std::rc::Weak<PreferencesBinding>, f: F)
where
    F: FnOnce(&PreferencesBinding) + 'static,
{
    let weak = weak.clone();
    glib::idle_add_local_once(move || {
        with_upgraded_weak(&weak, f);
    });
}

/// Route one widget gesture through the shared settings state machine.
/// The controller records update errors and refreshes views through the
/// existing observers; the headline is surfaced here.
fn send(binding: &Rc<PreferencesBinding>, event: SettingsEvent) {
    let effect = binding.controller.borrow_mut().update(event);
    debug_assert!(effect.is_none(), "gtk has no browse-directory button");
    binding.error_label.set_text(
        binding
            .controller
            .borrow()
            .error_message
            .clone()
            .unwrap_or_default()
            .as_str(),
    );
}

/// Wire a combo to one controller event: upgrade, guard, resolve the
/// shared choice, send. No selection is a no-op.
fn connect_choice<T>(
    combo: &gtk::ComboBoxText,
    weak: &std::rc::Weak<PreferencesBinding>,
    choices: impl Fn(&PreferencesBinding) -> Vec<ChoiceView<T>> + 'static,
    event: impl Fn(ChoiceView<T>) -> SettingsEvent + 'static,
) where
    T: Clone + PartialEq + Eq + 'static,
{
    combo.connect_changed({
        let w = weak.clone();
        move |combo| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() {
                return;
            }
            if let Some(choice) = active_choice_view(combo, &(choices)(&b)) {
                send(&b, event(choice));
            }
        }
    });
}

/// Wire a toggle to one controller event: upgrade, guard, send.
fn connect_toggle(
    button: &gtk::CheckButton,
    weak: &std::rc::Weak<PreferencesBinding>,
    event: impl Fn(bool) -> SettingsEvent + 'static,
) {
    button.connect_toggled({
        let w = weak.clone();
        move |button| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() {
                return;
            }
            send(&b, event(button.is_active()));
        }
    });
}

/// Wire a spin to one controller event; the caller casts the raw value.
fn connect_spin(
    spin: &gtk::SpinButton,
    weak: &std::rc::Weak<PreferencesBinding>,
    event: impl Fn(f64) -> SettingsEvent + 'static,
) {
    spin.connect_value_changed({
        let w = weak.clone();
        move |spin| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() {
                return;
            }
            send(&b, event(spin.value()));
        }
    });
}

fn combo_box(entries: &[(&str, &str)]) -> gtk::ComboBoxText {
    let combo = gtk::ComboBoxText::new();
    for (id, label) in entries {
        combo.append(Some(id), label);
    }
    combo
}

fn add_conflict_labels(view: &InputTabView, page: &gtk::Box) {
    for conflict in &view.conflicts {
        let label = gtk::Label::new(Some(&conflict.message));
        label.set_xalign(0.0);
        page.append(&label);
    }
}

fn connect_slot_combo(
    slot: &ControllerSlotView,
    language: AppLanguage,
    index: usize,
    self_weak: &std::rc::Weak<PreferencesBinding>,
) -> gtk::ComboBoxText {
    let combo = gtk::ComboBoxText::new();
    combo.append(Some("__none__"), ui_text(language, UiText::None));
    for choice in &slot.choices {
        if let Some(id) = &choice.value {
            combo.append(Some(id), &choice.label);
        }
    }
    combo.set_active_id(Some(
        slot.selected_profile_id.as_deref().unwrap_or("__none__"),
    ));
    combo.set_sensitive(!slot.occupied_by_other_slot);
    let slot_id = slot.slot_id;
    let weak = self_weak.clone();
    combo.connect_changed(move |combo| {
        let Some(binding) = weak.upgrade() else {
            return;
        };
        if binding.refreshing.get() {
            return;
        }
        let profile = combo
            .active_id()
            .and_then(|id| (id.as_str() != "__none__").then(|| id.to_string()));
        send(&binding, SettingsEvent::SelectInputTab(index));
        send(
            &binding,
            SettingsEvent::SetControllerSlot {
                slot: slot_id,
                controller_id: profile,
            },
        );
        if !binding
            .controller
            .borrow()
            .error_message
            .clone()
            .unwrap_or_default()
            .is_empty()
        {
            let idx = index;
            schedule_idle(&binding.self_weak, move |b| {
                b.with_refreshing(|| {
                    if let Some(vm) = b.controller.borrow().vm.inputs().get(idx) {
                        b.rebuild_input_page(idx, &vm.view.get());
                    }
                });
            });
        }
    });
    combo
}

/// GTK widget tests — only run on Linux with a display server.
/// macOS requires main-thread GTK init which Rust test harness cannot
/// guarantee. These tests compile on all platforms but only execute
/// when `gtk::init()` succeeds.
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use crate::test_support::ui_test_available;

    #[test]
    fn preferences_finish_runs_once_even_when_close_reenters() {
        let count = Rc::new(Cell::new(0));
        let callback: FinishCallback = Rc::new(RefCell::new(None));
        let callback_weak = Rc::downgrade(&callback);
        let count_on_close = count.clone();
        *callback.borrow_mut() = Some(Box::new(move || {
            count_on_close.set(count_on_close.get() + 1);
            finish_preferences(&callback_weak.upgrade().unwrap());
        }));
        finish_preferences(&callback);
        finish_preferences(&callback);
        assert_eq!(count.get(), 1);
    }

    #[test]
    fn with_mut_releases_borrow_before_follow_up_work() {
        let value = RefCell::new(0);
        with_mut(&value, |current| *current = 1);
        *value.borrow_mut() = 2;
        assert_eq!(*value.borrow(), 2);
    }

    macro_rules! gtk_test {
        ($name:ident, $body:expr) => {
            #[test]
            fn $name() {
                if ui_test_available() {
                    $body()
                } else {
                    eprintln!("skipped (no GTK display)");
                }
            }
        };
    }

    gtk_test!(preferences_native_close_and_destroy_finish_once, || {
        for destroy in [false, true] {
            let dialog = gtk::Dialog::builder().title("lifetime test").build();
            let count = Rc::new(Cell::new(0));
            let count_on_close = count.clone();
            let callback: FinishCallback = Rc::new(RefCell::new(Some(Box::new(move || {
                count_on_close.set(count_on_close.get() + 1);
            }))));
            connect_preferences_finish(&dialog, &callback);
            dialog.present();
            if destroy {
                dialog.destroy();
            } else {
                dialog.close();
            }
            assert_eq!(
                count.get(),
                1,
                "native close/destroy must release while references remain"
            );
            dialog.destroy();
            finish_preferences(&callback);
            assert_eq!(
                count.get(),
                1,
                "overlapping terminal signals never double release"
            );
        }
    });

    gtk_test!(refreshing_guard_prevents_reentrant_signal, || {
        let binding = create_test_binding();
        binding.with_refreshing(|| {
            let prev = binding.refreshing.get();
            assert!(prev, "refreshing should be true during guard");
        });
        assert!(
            !binding.refreshing.get(),
            "refreshing should be false after guard"
        );
    });

    gtk_test!(send_helper_clears_error_on_success, || {
        let binding = create_test_binding();
        send(&binding, SettingsEvent::ToggleMute(true));
        assert!(
            binding.error_label.text().is_empty(),
            "successful update should clear the error label"
        );
        assert!(
            binding.controller.borrow().vm.audio.view.get().muted,
            "mute toggle should reach the view model"
        );
    });

    gtk_test!(general_widgets_initial_values_match_viewmodel, || {
        let binding = create_test_binding();
        let gv = binding.controller.borrow().vm.general.view.get();
        assert_eq!(
            binding.general.language_combo.active(),
            choice_position(&gv.language_choices, &gv.language)
        );
    });

    gtk_test!(ok_button_sensitivity_matches_validation, || {
        let binding = create_test_binding();
        let ok = binding.controller.borrow().validation_banner().can_submit;
        assert_eq!(binding.ok_button.is_sensitive(), ok);
    });

    // Pure with_upgraded_weak tests — no GTK needed, run on all platforms.
    #[test]
    fn with_upgraded_weak_calls_closure_when_alive() {
        use std::rc::Rc;
        let rc = Rc::new(42);
        let weak = Rc::downgrade(&rc);
        let mut value = 0;
        let called = with_upgraded_weak(&weak, |v| value = *v);
        assert!(called);
        assert_eq!(value, 42);
    }

    #[test]
    fn with_upgraded_weak_skips_closure_after_drop() {
        use std::rc::{Rc, Weak};
        let weak: Weak<i32> = {
            let rc = Rc::new(42);
            Rc::downgrade(&rc)
        };
        let mut executed = false;
        let called = with_upgraded_weak(&weak, |_| executed = true);
        assert!(!called, "should not execute closure after drop");
        assert!(!executed, "closure should not run after weak is dropped");
    }

    #[test]
    fn with_upgraded_weak_returns_false_for_null_weak() {
        use std::rc::Weak;
        let weak: Weak<i32> = Weak::new();
        let called = with_upgraded_weak(&weak, |_| {});
        assert!(!called);
    }

    fn create_test_binding() -> Rc<PreferencesBinding> {
        use std::sync::Arc;

        let snapshot = nerust_settings_core::model::snapshot::SettingsSnapshot {
            shared: nerust_settings_core::model::shared::DesktopSharedSettings::default(),
            local: nerust_settings_core::model::local::HostBackendLocalSettings::default(),
            app_state: nerust_settings_core::model::app_state::DesktopAppState::default(),
        };
        let registry = Arc::new(nerust_gui_shell::registry::SystemRegistry::new(Vec::new()));
        let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());

        let controller = SettingsController::new(
            &snapshot,
            Arc::clone(&registry),
            Arc::clone(&audio_registry),
        );

        let dialog = gtk::Dialog::builder().title("test").build();
        let stack = gtk::Stack::new();
        let ok_button = gtk::Button::with_label("OK");
        let error_label = gtk::Label::new(None);

        PreferencesBinding::new(
            controller,
            PreferencesWidgets {
                general: GeneralWidgets {
                    language_combo: gtk::ComboBoxText::new(),
                    storage_policy_combo: gtk::ComboBoxText::new(),
                    storage_dir_entry: gtk::Entry::new(),
                    storage_dir_row: gtk::Box::new(gtk::Orientation::Horizontal, 0),
                    storage_error_label: gtk::Label::new(None),
                    language_label: gtk::Label::new(None),
                    storage_policy_label: gtk::Label::new(None),
                    storage_dir_label: gtk::Label::new(None),
                },
                video: VideoWidgets {
                    fullscreen_check: gtk::CheckButton::new(),
                    scaling_combo: gtk::ComboBoxText::new(),
                    scaling_label: gtk::Label::new(None),
                    vsync_check: gtk::CheckButton::new(),
                },
                audio: AudioWidgets {
                    mute_check: gtk::CheckButton::new(),
                    volume_spin: gtk::SpinButton::with_range(0.0, 100.0, 1.0),
                    volume_label: gtk::Label::new(None),
                    sample_rate_combo: gtk::ComboBoxText::new(),
                    sample_rate_label: gtk::Label::new(None),
                    latency_spin: gtk::SpinButton::with_range(10.0, 200.0, 1.0),
                    latency_label: gtk::Label::new(None),
                },
                notifications: NotificationWidgets {
                    info_check: gtk::CheckButton::new(),
                    info_duration_spin: gtk::SpinButton::with_range(1000.0, 30000.0, 500.0),
                    info_color_combo: gtk::ComboBoxText::new(),
                    warning_check: gtk::CheckButton::new(),
                    warning_duration_spin: gtk::SpinButton::with_range(1000.0, 30000.0, 500.0),
                    warning_color_combo: gtk::ComboBoxText::new(),
                    error_check: gtk::CheckButton::new(),
                    error_modal_radio: gtk::CheckButton::new(),
                    error_onscreen_radio: gtk::CheckButton::new(),
                    error_color_combo: gtk::ComboBoxText::new(),
                    status_check: gtk::CheckButton::new(),
                    position_combo: gtk::ComboBoxText::new(),
                    status_position_combo: gtk::ComboBoxText::new(),
                    area_combo: gtk::ComboBoxText::new(),
                    status_area_combo: gtk::ComboBoxText::new(),
                },
                input: InputTabWidgets {
                    _notebook: gtk::Notebook::new(),
                    pages: vec![],
                    section_notebooks: RefCell::new(vec![]),
                    shortcuts_page: gtk::Box::new(gtk::Orientation::Vertical, 6),
                    shortcuts_tab_label: gtk::Label::new(None),
                },
                system: SystemTabWidgets {
                    _notebook: gtk::Notebook::new(),
                    pages: vec![],
                },
                ok_button: ok_button.clone().upcast::<gtk::Widget>(),
                cancel_button: gtk::Button::new().upcast::<gtk::Widget>(),
                error_label: error_label.clone(),
                dialog: dialog.clone(),
                stack: stack.clone(),
                page_ids: vec![],
            },
        )
    }
}
