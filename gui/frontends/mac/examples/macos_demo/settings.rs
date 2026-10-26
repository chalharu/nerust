//! Settings window: six tabs bound to the shared `SettingsController`.
//!
//! An `NSTabView` mirrors the tao sidebar (General/Input/Video/Audio/
//! Notifications/System); combos and checkboxes update the controller live
//! through tag-dispatched delegate actions, text fields commit on Enter
//! or OK, and OK stages through the mailbox protocol.

use nerust_frontend_host::settings::{SettingsController, SettingsEvent};
use nerust_gui_viewmodel::settings::dto::ChoiceView;
use objc2::{MainThreadMarker, msg_send, rc::Retained, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSComboBox, NSControlStateValueOn, NSTabView, NSTabViewItem,
    NSTextField, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

/// Control tags: one delegate action per kind, dispatched by tag.
mod tag {
    pub const LANG: isize = 1;
    pub const STORAGE_POLICY: isize = 2;
    pub const SAMPLE_RATE: isize = 3;
    pub const FULLSCREEN: isize = 10;
    pub const VSYNC: isize = 11;
    pub const MUTE: isize = 12;
    pub const INFO_ENABLED: isize = 20;
    pub const INFO_COLOR: isize = 21;
    pub const WARNING_ENABLED: isize = 22;
    pub const WARNING_COLOR: isize = 23;
    pub const ERROR_ENABLED: isize = 24;
    pub const ERROR_DISPLAY: isize = 25;
    pub const ERROR_COLOR: isize = 26;
    pub const STATUS_ENABLED: isize = 27;
    pub const POSITION: isize = 28;
    pub const AREA_MODE: isize = 29;
    pub const STATUS_POSITION: isize = 30;
    pub const STATUS_AREA_MODE: isize = 31;
    pub const SCALING: isize = 40;
}

pub struct SettingsDlg {
    pub window: Retained<NSWindow>,
    controller: SettingsController,
    tabs: Retained<NSTabView>,
    banner: Retained<NSTextField>,
    ok_btn: Retained<NSButton>,
    lang_choices: Vec<ChoiceView<nerust_settings_core::model::language::AppLanguage>>,
    rate_choices: Vec<ChoiceView<u32>>,
    storage_choices: Vec<ChoiceView<nerust_settings_core::model::shared::StoragePolicy>>,
    scaling_choices: Vec<ChoiceView<nerust_settings_core::model::local::ScalingMode>>,
    color_choices: Vec<ChoiceView<nerust_settings_core::model::local::NotificationColor>>,
    position_choices: Vec<ChoiceView<nerust_settings_core::model::local::NotificationPosition>>,
    area_choices: Vec<ChoiceView<nerust_settings_core::model::local::NotificationAreaMode>>,
    slot_combos: Vec<SlotCombo>,
    binding_rows: Vec<nerust_settings_core::editor::CaptureTarget>,
    system_combos: Vec<SystemCombo>,
    dir_field: Option<Retained<NSTextField>>,
    volume_field: Option<Retained<NSTextField>>,
    latency_field: Option<Retained<NSTextField>>,
    info_dur_field: Option<Retained<NSTextField>>,
    warn_dur_field: Option<Retained<NSTextField>>,
}

struct SlotCombo {
    slot: nerust_input_traits::AttachmentId,
    choices: Vec<ChoiceView<Option<String>>>,
}

struct SystemCombo {
    field: nerust_system_traits::factory::descriptor::SystemSettingsFieldId,
    choices: Vec<ChoiceView<nerust_system_traits::factory::descriptor::SystemSettingsChoiceId>>,
}

fn make_label(mtm: MainThreadMarker, parent: &NSView, x: f64, y: f64, w: f64, text: &str) {
    let label: Retained<NSTextField> = NSTextField::initWithFrame(
        mtm.alloc::<NSTextField>(),
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, 20.0)),
    );
    label.setStringValue(&NSString::from_str(text));
    label.setEditable(false);
    label.setBezeled(false);
    label.setDrawsBackground(false);
    parent.addSubview(&label);
}

#[allow(clippy::too_many_arguments)]
fn make_combo(
    mtm: MainThreadMarker,
    parent: &NSView,
    delegate: &super::app::AppDelegate,
    x: f64,
    y: f64,
    w: f64,
    tag: isize,
    action: objc2::runtime::Sel,
) -> Retained<NSComboBox> {
    let combo: Retained<NSComboBox> = NSComboBox::initWithFrame(
        mtm.alloc::<NSComboBox>(),
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, 24.0)),
    );
    unsafe {
        combo.setTarget(Some(super::app::anyobject(delegate)));
        combo.setAction(Some(action));
    }
    combo.setTag(tag);
    parent.addSubview(&combo);
    combo
}

#[allow(clippy::too_many_arguments)]
fn make_check(
    mtm: MainThreadMarker,
    parent: &NSView,
    delegate: &super::app::AppDelegate,
    x: f64,
    y: f64,
    w: f64,
    text: &str,
    tag: isize,
) -> Retained<NSButton> {
    let check: Retained<NSButton> = unsafe {
        NSButton::checkboxWithTitle_target_action(
            &NSString::from_str(text),
            Some(super::app::anyobject(delegate)),
            Some(sel!(settingsCheck:)),
            mtm,
        )
    };
    check.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(w, 22.0)));
    check.setTag(tag);
    parent.addSubview(&check);
    check
}

#[allow(clippy::too_many_arguments)]
fn make_field(
    mtm: MainThreadMarker,
    parent: &NSView,
    x: f64,
    y: f64,
    w: f64,
    placeholder: &str,
    tag: isize,
    delegate: &super::app::AppDelegate,
) -> Retained<NSTextField> {
    let field: Retained<NSTextField> = NSTextField::initWithFrame(
        mtm.alloc::<NSTextField>(),
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, 22.0)),
    );
    field.setPlaceholderString(Some(&NSString::from_str(placeholder)));
    unsafe {
        field.setTarget(Some(super::app::anyobject(delegate)));
        field.setAction(Some(sel!(settingsField:)));
    }
    field.setTag(tag);
    parent.addSubview(&field);
    field
}

#[allow(clippy::too_many_arguments)]
fn make_button(
    mtm: MainThreadMarker,
    delegate: &super::app::AppDelegate,
    y: f64,
    x: f64,
    title: &str,
    action: objc2::runtime::Sel,
) -> Retained<NSButton> {
    let btn: Retained<NSButton> = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(super::app::anyobject(delegate)),
            Some(action),
            mtm,
        )
    };
    btn.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(90.0, 24.0)));
    btn
}

fn fill_combo<T: Clone + PartialEq + Eq>(
    combo: &NSComboBox,
    choices: &[ChoiceView<T>],
    current: &T,
) {
    combo.removeAllItems();
    for choice in choices {
        unsafe {
            combo.addItemWithObjectValue(&NSString::from_str(&choice.label));
        }
    }
    if let Some(i) = choices.iter().position(|c| &c.value == current) {
        combo.selectItemAtIndex(i as isize);
    }
}

fn page(
    mtm: MainThreadMarker,
    tabs: &NSTabView,
    id: &str,
    title: &str,
    h: f64,
) -> Retained<NSView> {
    let view: Retained<NSView> = NSView::initWithFrame(
        mtm.alloc::<NSView>(),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(660.0, h)),
    );
    let item: Retained<NSTabViewItem> = unsafe {
        let ident = NSString::from_str(id);
        msg_send![mtm.alloc::<NSTabViewItem>(), initWithIdentifier: &*ident]
    };
    item.setLabel(&NSString::from_str(title));
    item.setView(Some(&view));
    tabs.addTabViewItem(&item);
    view
}

impl SettingsDlg {
    pub fn open(
        mtm: MainThreadMarker,
        host: &mut nerust_mac::MacHost,
        delegate: &super::app::AppDelegate,
    ) -> Option<Self> {
        let controller = host.open_settings();
        let frame = NSRect::new(NSPoint::new(200.0, 160.0), NSSize::new(700.0, 600.0));
        let window: Retained<NSWindow> = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc::<NSWindow>(),
                frame,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&NSString::from_str("Settings"));
        let tabs: Retained<NSTabView> = NSTabView::initWithFrame(
            mtm.alloc::<NSTabView>(),
            NSRect::new(NSPoint::new(8.0, 60.0), NSSize::new(684.0, 500.0)),
        );
        if let Some(view) = window.contentView() {
            view.addSubview(&tabs);
        }
        let banner: Retained<NSTextField> = NSTextField::initWithFrame(
            mtm.alloc::<NSTextField>(),
            NSRect::new(NSPoint::new(8.0, 34.0), NSSize::new(500.0, 20.0)),
        );
        banner.setEditable(false);
        banner.setBezeled(false);
        banner.setDrawsBackground(false);
        if let Some(view) = window.contentView() {
            view.addSubview(&banner);
        }
        let ok_btn = make_button(mtm, delegate, 8.0, 500.0, "OK", sel!(submitSettings:));
        if let Some(view) = window.contentView() {
            view.addSubview(&ok_btn);
            let cancel = make_button(mtm, delegate, 8.0, 600.0, "Cancel", sel!(cancelSettings:));
            view.addSubview(&cancel);
        }
        window.makeKeyAndOrderFront(None);
        let mut dlg = Self {
            window,
            controller,
            tabs,
            banner,
            ok_btn,
            lang_choices: Vec::new(),
            rate_choices: Vec::new(),
            storage_choices: Vec::new(),
            scaling_choices: Vec::new(),
            color_choices: Vec::new(),
            position_choices: Vec::new(),
            area_choices: Vec::new(),
            slot_combos: Vec::new(),
            binding_rows: Vec::new(),
            system_combos: Vec::new(),
            dir_field: None,
            volume_field: None,
            latency_field: None,
            info_dur_field: None,
            warn_dur_field: None,
        };
        dlg.build_pages(mtm, delegate);
        dlg.refresh_chrome();
        Some(dlg)
    }

    pub(crate) fn build_pages(
        &mut self,
        mtm: MainThreadMarker,
        delegate: &super::app::AppDelegate,
    ) {
        while self.tabs.numberOfTabViewItems() > 0 {
            let item = self.tabs.tabViewItemAtIndex(0);
            self.tabs.removeTabViewItem(&item);
        }
        let gv = self.controller.vm.general.view.get();
        let vv = self.controller.vm.video.view.get();
        let av = self.controller.vm.audio.view.get();
        let nv = self.controller.vm.notifications.view.get();
        self.lang_choices = gv.language_choices.clone();
        self.storage_choices = gv.storage_policy_choices.clone();
        self.scaling_choices = vv.scaling_choices.clone();
        self.rate_choices = av.sample_rate_choices.clone();
        self.color_choices = nv.color_choices.clone();
        self.position_choices = nv.position_choices.clone();
        self.area_choices = nv.area_mode_choices.clone();

        // General.
        let general = page(mtm, &self.tabs, "general", "General", 460.0);
        make_label(mtm, &general, 12.0, 420.0, 140.0, "Language:");
        let combo = make_combo(
            mtm,
            &general,
            delegate,
            160.0,
            418.0,
            240.0,
            tag::LANG,
            sel!(settingsCombo:),
        );
        fill_combo(&combo, &self.lang_choices, &gv.language);
        make_label(mtm, &general, 12.0, 380.0, 140.0, "Save storage:");
        let combo = make_combo(
            mtm,
            &general,
            delegate,
            160.0,
            378.0,
            240.0,
            tag::STORAGE_POLICY,
            sel!(settingsCombo:),
        );
        fill_combo(&combo, &self.storage_choices, &gv.storage_policy);
        if gv.show_storage_directory {
            make_label(mtm, &general, 12.0, 340.0, 140.0, "Directory:");
            let dir = make_field(
                mtm,
                &general,
                160.0,
                338.0,
                280.0,
                "Custom directory",
                1,
                delegate,
            );
            dir.setStringValue(&NSString::from_str(&gv.storage_directory));
            self.dir_field = Some(dir);
            let browse: Retained<NSButton> = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str("Browse..."),
                    Some(super::app::anyobject(delegate)),
                    Some(sel!(settingsBrowse:)),
                    mtm,
                )
            };
            browse.setFrame(NSRect::new(
                NSPoint::new(450.0, 336.0),
                NSSize::new(110.0, 26.0),
            ));
            general.addSubview(&browse);
            if let Some(error) = self.controller.storage_error() {
                make_label(mtm, &general, 12.0, 308.0, 540.0, &error);
            }
        }
        // Video.
        let video = page(mtm, &self.tabs, "video", "Video", 460.0);
        let check = make_check(
            mtm,
            &video,
            delegate,
            12.0,
            420.0,
            280.0,
            "Fullscreen at startup",
            tag::FULLSCREEN,
        );
        if vv.fullscreen_default {
            check.setState(NSControlStateValueOn);
        }
        make_label(mtm, &video, 12.0, 380.0, 140.0, "Scaling:");
        let combo = make_combo(
            mtm,
            &video,
            delegate,
            160.0,
            378.0,
            240.0,
            tag::SCALING,
            sel!(settingsCombo:),
        );
        fill_combo(&combo, &self.scaling_choices, &vv.scaling);
        let check = make_check(
            mtm,
            &video,
            delegate,
            12.0,
            340.0,
            200.0,
            "VSync",
            tag::VSYNC,
        );
        if vv.vsync {
            check.setState(NSControlStateValueOn);
        }
        // Audio.
        let audio = page(mtm, &self.tabs, "audio", "Audio", 460.0);
        let check = make_check(mtm, &audio, delegate, 12.0, 420.0, 200.0, "Mute", tag::MUTE);
        if av.muted {
            check.setState(NSControlStateValueOn);
        }
        make_label(
            mtm,
            &audio,
            12.0,
            380.0,
            140.0,
            &format!("Volume: {}%", av.volume_percent),
        );
        let volume = make_field(mtm, &audio, 160.0, 378.0, 100.0, "0-100", 2, delegate);
        volume.setStringValue(&NSString::from_str(&av.volume_percent.to_string()));
        self.volume_field = Some(volume);
        make_label(mtm, &audio, 12.0, 340.0, 140.0, "Sample rate:");
        let combo = make_combo(
            mtm,
            &audio,
            delegate,
            160.0,
            338.0,
            240.0,
            tag::SAMPLE_RATE,
            sel!(settingsCombo:),
        );
        fill_combo(&combo, &self.rate_choices, &av.sample_rate);
        make_label(mtm, &audio, 12.0, 300.0, 140.0, "Latency ms:");
        let latency = make_field(mtm, &audio, 160.0, 298.0, 100.0, "10-200", 3, delegate);
        latency.setStringValue(&NSString::from_str(&av.latency_ms.to_string()));
        self.latency_field = Some(latency);
        // Notifications.
        let notif = page(mtm, &self.tabs, "notifications", "Notifications", 460.0);
        let mut y = 420.0;
        for (title, enabled, duration, color, color_tag, en_tag) in [
            (
                "Info",
                nv.info_enabled,
                nv.info_duration_ms,
                nv.info_color,
                tag::INFO_COLOR,
                tag::INFO_ENABLED,
            ),
            (
                "Warning",
                nv.warning_enabled,
                nv.warning_duration_ms,
                nv.warning_color,
                tag::WARNING_COLOR,
                tag::WARNING_ENABLED,
            ),
        ] {
            make_label(mtm, &notif, 12.0, y, 200.0, title);
            y -= 26.0;
            let check = make_check(mtm, &notif, delegate, 24.0, y, 200.0, "Enabled", en_tag);
            if enabled {
                check.setState(NSControlStateValueOn);
            }
            make_label(mtm, &notif, 240.0, y + 2.0, 100.0, "Duration ms:");
            let dur_tag = if color_tag == tag::INFO_COLOR { 4 } else { 5 };
            let dur = make_field(mtm, &notif, 350.0, y, 90.0, "1000-30000", dur_tag, delegate);
            dur.setStringValue(&NSString::from_str(&duration.to_string()));
            if color_tag == tag::INFO_COLOR {
                self.info_dur_field = Some(dur);
            } else {
                self.warn_dur_field = Some(dur);
            }
            y -= 26.0;
            make_label(mtm, &notif, 24.0, y + 2.0, 80.0, "Color:");
            let combo = make_combo(
                mtm,
                &notif,
                delegate,
                110.0,
                y,
                180.0,
                color_tag,
                sel!(settingsCombo:),
            );
            fill_combo(&combo, &self.color_choices, &color);
            y -= 30.0;
        }
        make_label(mtm, &notif, 12.0, y, 200.0, "Error");
        y -= 26.0;
        let check = make_check(
            mtm,
            &notif,
            delegate,
            24.0,
            y,
            200.0,
            "Enabled",
            tag::ERROR_ENABLED,
        );
        if nv.error_enabled {
            check.setState(NSControlStateValueOn);
        }
        y -= 26.0;
        make_label(mtm, &notif, 24.0, y + 2.0, 120.0, "Display:");
        let combo = make_combo(
            mtm,
            &notif,
            delegate,
            150.0,
            y,
            180.0,
            tag::ERROR_DISPLAY,
            sel!(settingsCombo:),
        );
        for name in ["Modal dialog", "On-screen"] {
            unsafe {
                combo.addItemWithObjectValue(&NSString::from_str(name));
            }
        }
        combo.selectItemAtIndex(if nv.error_modal { 0 } else { 1 });
        y -= 26.0;
        make_label(mtm, &notif, 24.0, y + 2.0, 80.0, "Color:");
        let combo = make_combo(
            mtm,
            &notif,
            delegate,
            110.0,
            y,
            180.0,
            tag::ERROR_COLOR,
            sel!(settingsCombo:),
        );
        fill_combo(&combo, &self.color_choices, &nv.error_color);
        y -= 30.0;
        let check = make_check(
            mtm,
            &notif,
            delegate,
            12.0,
            y,
            240.0,
            "Status line enabled",
            tag::STATUS_ENABLED,
        );
        if nv.status_enabled {
            check.setState(NSControlStateValueOn);
        }
        // Input.
        let input = page(mtm, &self.tabs, "input", "Input", 460.0);
        let mut y = 420.0;
        self.slot_combos.clear();
        self.binding_rows.clear();
        let mut combo_tag = 100;
        let inputs: Vec<_> = self
            .controller
            .vm
            .inputs()
            .iter()
            .map(|tab| tab.view.get())
            .collect();
        for view in &inputs {
            make_label(mtm, &input, 12.0, y, 300.0, &view.label);
            y -= 24.0;
            for slot in &view.slots {
                make_label(mtm, &input, 24.0, y + 2.0, 150.0, &slot.label);
                let combo = make_combo(
                    mtm,
                    &input,
                    delegate,
                    180.0,
                    y,
                    220.0,
                    combo_tag,
                    sel!(settingsCombo:),
                );
                fill_combo(&combo, &slot.choices, &slot.selected_profile_id);
                self.slot_combos.push(SlotCombo {
                    slot: slot.slot_id,
                    choices: slot.choices.clone(),
                });
                combo_tag += 1;
                if slot.occupied_by_other_slot {
                    make_label(mtm, &input, 410.0, y + 2.0, 120.0, "(occupied)");
                }
                y -= 26.0;
            }
            for section in &view.sections {
                make_label(mtm, &input, 24.0, y, 300.0, &section.label);
                y -= 22.0;
                for row in &section.rows {
                    self.binding_row(
                        mtm,
                        delegate,
                        &input,
                        &mut y,
                        &row.label,
                        &row_value(&row.value),
                        row.target.clone(),
                    );
                    if y < 30.0 {
                        break;
                    }
                }
            }
            for conflict in &view.conflicts {
                make_label(mtm, &input, 24.0, y, 500.0, &conflict.message);
                y -= 20.0;
            }
            if y < 30.0 {
                break;
            }
        }
        make_label(mtm, &input, 12.0, y, 300.0, "Shortcuts");
        y -= 22.0;
        let shortcuts = self.controller.vm.shortcuts.view.get().clone();
        for row in &shortcuts.rows {
            self.binding_row(
                mtm,
                delegate,
                &input,
                &mut y,
                &row.label,
                &row_value(&row.value),
                row.target.clone(),
            );
            if y < 30.0 {
                break;
            }
        }
        let capture = self.controller.vm.capture.view.get();
        make_label(mtm, &input, 12.0, 8.0, 500.0, &capture.prompt);
        // System.
        let system = page(mtm, &self.tabs, "system", "System", 460.0);
        let mut y = 420.0;
        self.system_combos.clear();
        let mut sys_tag = 300;
        let systems: Vec<_> = self
            .controller
            .vm
            .systems()
            .iter()
            .map(|tab| tab.view.get())
            .collect();
        for view in &systems {
            make_label(mtm, &system, 12.0, y, 300.0, &view.label);
            y -= 24.0;
            for field in &view.fields {
                make_label(mtm, &system, 24.0, y + 2.0, 200.0, &field.label);
                let combo = make_combo(
                    mtm,
                    &system,
                    delegate,
                    230.0,
                    y,
                    220.0,
                    sys_tag,
                    sel!(settingsCombo:),
                );
                fill_combo(&combo, &field.choices, &field.selected);
                self.system_combos.push(SystemCombo {
                    field: field.id.clone(),
                    choices: field.choices.clone(),
                });
                sys_tag += 1;
                y -= 26.0;
                if y < 30.0 {
                    break;
                }
            }
            if y < 30.0 {
                break;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn binding_row(
        &mut self,
        mtm: MainThreadMarker,
        delegate: &super::app::AppDelegate,
        parent: &NSView,
        y: &mut f64,
        name: &str,
        value: &str,
        target: nerust_settings_core::editor::CaptureTarget,
    ) {
        make_label(mtm, parent, 36.0, *y, 200.0, &format!("{name}: {value}"));
        let change: Retained<NSButton> = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Change"),
                Some(super::app::anyobject(delegate)),
                Some(sel!(settingsBindingChange:)),
                mtm,
            )
        };
        let index = self.binding_rows.len() as isize;
        self.binding_rows.push(target);
        change.setFrame(NSRect::new(
            NSPoint::new(250.0, *y - 2.0),
            NSSize::new(90.0, 24.0),
        ));
        change.setTag(1000 + index);
        parent.addSubview(&change);
        let clear: Retained<NSButton> = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Clear"),
                Some(super::app::anyobject(delegate)),
                Some(sel!(settingsBindingClear:)),
                mtm,
            )
        };
        clear.setFrame(NSRect::new(
            NSPoint::new(346.0, *y - 2.0),
            NSSize::new(90.0, 24.0),
        ));
        clear.setTag(2000 + index);
        parent.addSubview(&clear);
        *y -= 26.0;
    }

    fn update(&mut self, event: SettingsEvent) {
        let _ = self.controller.update(event);
        self.refresh_chrome();
    }

    pub fn is_capturing(&self) -> bool {
        self.controller.vm.capture.view.get().target.is_some()
    }

    pub fn capture_key(&mut self, key: nerust_keyboard::Key) {
        self.update(SettingsEvent::CaptureKey(key));
    }

    pub fn field_committed_storage(&mut self, path: std::path::PathBuf) {
        self.update(SettingsEvent::StorageDirectorySelected(Some(path)));
    }

    /// Control tag of the committed text field → settings event.
    pub fn field_committed(&mut self, tag: isize, text: &str) {
        let event = match tag {
            1 => SettingsEvent::SetStorageDirectory(text.to_string()),
            2 => match text.trim().parse::<u8>() {
                Ok(value) => SettingsEvent::SetVolume(value),
                Err(_) => return,
            },
            3 => match text.trim().parse::<u16>() {
                Ok(value) => SettingsEvent::SetLatency(value),
                Err(_) => return,
            },
            4 => match text.trim().parse::<u32>() {
                Ok(value) => SettingsEvent::SetInfoDuration(value),
                Err(_) => return,
            },
            5 => match text.trim().parse::<u32>() {
                Ok(value) => SettingsEvent::SetWarningDuration(value),
                Err(_) => return,
            },
            _ => return,
        };
        self.update(event);
    }

    /// Combo selection by tag: fixed pages, per-slot and per-field combos.
    pub fn combo_changed(&mut self, tag: isize, index: usize) {
        let event =
            match tag {
                tag::LANG => {
                    Self::selected_at(&self.lang_choices, index).map(SettingsEvent::SetLanguage)
                }
                tag::STORAGE_POLICY => Self::selected_at(&self.storage_choices, index)
                    .map(SettingsEvent::SetStoragePolicy),
                tag::SAMPLE_RATE => {
                    Self::selected_at(&self.rate_choices, index).map(SettingsEvent::SetSampleRate)
                }
                tag::SCALING => {
                    Self::selected_at(&self.scaling_choices, index).map(SettingsEvent::SetScaling)
                }
                tag::INFO_COLOR => {
                    Self::selected_at(&self.color_choices, index).map(SettingsEvent::SetInfoColor)
                }
                tag::WARNING_COLOR => Self::selected_at(&self.color_choices, index)
                    .map(SettingsEvent::SetWarningColor),
                tag::ERROR_COLOR => {
                    Self::selected_at(&self.color_choices, index).map(SettingsEvent::SetErrorColor)
                }
                tag::ERROR_DISPLAY => Some(SettingsEvent::ToggleErrorModal(index == 0)),
                tag::POSITION => {
                    Self::selected_at(&self.position_choices, index).map(SettingsEvent::SetPosition)
                }
                tag::STATUS_POSITION => Self::selected_at(&self.position_choices, index)
                    .map(SettingsEvent::SetStatusPosition),
                tag::AREA_MODE => {
                    Self::selected_at(&self.area_choices, index).map(SettingsEvent::SetAreaMode)
                }
                tag::STATUS_AREA_MODE => Self::selected_at(&self.area_choices, index)
                    .map(SettingsEvent::SetStatusAreaMode),
                _ if (100..300).contains(&tag) => {
                    self.slot_combos.get((tag - 100) as usize).and_then(|slot| {
                        slot.choices.get(index).cloned().map(|choice| {
                            SettingsEvent::SetControllerSlot {
                                slot: slot.slot,
                                controller_id: choice.value,
                            }
                        })
                    })
                }
                _ => self
                    .system_combos
                    .get((tag - 300) as usize)
                    .and_then(|sys| {
                        sys.choices.get(index).cloned().map(|choice| {
                            SettingsEvent::SetSystemChoice(sys.field.0.to_string(), choice)
                        })
                    }),
            };
        if let Some(event) = event {
            self.update(event);
        }
    }

    fn selected_at<T: Clone + PartialEq + Eq>(
        choices: &[ChoiceView<T>],
        index: usize,
    ) -> Option<ChoiceView<T>> {
        choices.get(index).cloned()
    }

    /// Checkbox by tag.
    pub fn check_changed(&mut self, tag: isize, on: bool) {
        let event = match tag {
            tag::FULLSCREEN => SettingsEvent::ToggleFullscreenDefault(on),
            tag::VSYNC => SettingsEvent::ToggleVsync(on),
            tag::MUTE => SettingsEvent::ToggleMute(on),
            tag::INFO_ENABLED => SettingsEvent::ToggleInfoEnabled(on),
            tag::WARNING_ENABLED => SettingsEvent::ToggleWarningEnabled(on),
            tag::ERROR_ENABLED => SettingsEvent::ToggleErrorEnabled(on),
            tag::STATUS_ENABLED => SettingsEvent::ToggleStatusEnabled(on),
            _ => return,
        };
        self.update(event);
    }

    /// Binding Change/Clear buttons by tag (1000+index / 2000+index).
    pub fn binding_change(&mut self, tag: isize) {
        if let Some(target) = self.binding_rows.get((tag - 1000) as usize).cloned() {
            self.update(SettingsEvent::StartCapture(target));
        }
    }

    pub fn binding_clear(&mut self, tag: isize) {
        if let Some(target) = self.binding_rows.get((tag - 2000) as usize).cloned() {
            self.update(SettingsEvent::ClearCapture(target));
        }
    }

    fn refresh_chrome(&mut self) {
        let banner = self.controller.validation_banner();
        self.banner.setStringValue(&NSString::from_str(
            banner.headline.as_deref().unwrap_or(""),
        ));
        self.ok_btn.setEnabled(banner.can_submit);
    }

    pub fn submit(&mut self, host: &mut nerust_mac::MacHost) -> bool {
        // Commit staged text fields (combos/checkboxes already live).
        if let Some(dir) = self.dir_field.as_ref() {
            self.field_committed(1, &dir.stringValue().to_string());
        }
        if let Some(volume) = self.volume_field.as_ref() {
            self.field_committed(2, &volume.stringValue().to_string());
        }
        if let Some(latency) = self.latency_field.as_ref() {
            self.field_committed(3, &latency.stringValue().to_string());
        }
        if let Some(dur) = self.info_dur_field.as_ref() {
            self.field_committed(4, &dur.stringValue().to_string());
        }
        if let Some(dur) = self.warn_dur_field.as_ref() {
            self.field_committed(5, &dur.stringValue().to_string());
        }
        let _ = self.controller.update(SettingsEvent::Submit);
        match host.close_settings(&self.controller) {
            Ok(true) => true,
            _ => {
                self.refresh_chrome();
                false
            }
        }
    }

    pub fn cancel(&mut self, host: &mut nerust_mac::MacHost) {
        let _ = host.close_settings(&self.controller);
    }

    pub fn close(&self) {
        self.window.close();
    }
}

fn row_value(value: &nerust_gui_viewmodel::settings::dto::BindingValueView) -> String {
    use nerust_gui_viewmodel::settings::dto::BindingValueView;
    match value {
        BindingValueView::Unbound(text)
        | BindingValueView::Bound(text)
        | BindingValueView::Capturing(text) => text.clone(),
    }
}

/// Headless verification hook mirroring the Win32 twin.
pub fn drive_submit() -> bool {
    let applied = super::app::with_settings(|dlg, host| dlg.submit(host)).unwrap_or(false);
    if applied {
        super::app::with_settings(|dlg, _| {
            dlg.close();
        });
    }
    applied
}
