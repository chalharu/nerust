//! Settings sheet: language and sample-rate combos, fullscreen checkbox.
//!
//! A plain `NSWindow` bound to the shared `SettingsController`, mirroring
//! the Win32 twin: OK stages through the mailbox protocol, Cancel closes
//! without applying. The error label shows the banner headline.

use nerust_frontend_host::settings::{SettingsController, SettingsEvent};
use nerust_gui_viewmodel::settings::dto::ChoiceView;
use nerust_settings_core::model::language::AppLanguage;
use objc2::{MainThreadMarker, rc::Retained, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSComboBox, NSControlStateValueOn, NSTextField, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

pub struct SettingsDlg {
    pub window: Retained<NSWindow>,
    pub(crate) controller: SettingsController,
    lang_combo: Retained<NSComboBox>,
    rate_combo: Retained<NSComboBox>,
    fs_check: Retained<NSButton>,
    err_label: Retained<NSTextField>,
    lang_choices: Vec<ChoiceView<AppLanguage>>,
    rate_choices: Vec<ChoiceView<u32>>,
}

impl SettingsDlg {
    pub fn open(
        mtm: MainThreadMarker,
        host: &mut nerust_mac::MacHost,
        delegate: &super::app::AppDelegate,
    ) -> Option<Self> {
        let controller = host.open_settings();
        let frame = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(420.0, 260.0));
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
        let label = |y: f64, text: &str| {
            let label: Retained<NSTextField> = NSTextField::initWithFrame(
                mtm.alloc::<NSTextField>(),
                NSRect::new(NSPoint::new(12.0, y), NSSize::new(120.0, 22.0)),
            );
            label.setStringValue(&NSString::from_str(text));
            label.setEditable(false);
            label.setBezeled(false);
            label.setDrawsBackground(false);
            if let Some(view) = window.contentView() {
                view.addSubview(&label);
            }
        };
        label(200.0, "Language:");
        label(170.0, "Sample rate:");
        let combo = |y: f64| {
            let combo: Retained<NSComboBox> = NSComboBox::initWithFrame(
                mtm.alloc::<NSComboBox>(),
                NSRect::new(NSPoint::new(140.0, y), NSSize::new(240.0, 24.0)),
            );
            if let Some(view) = window.contentView() {
                view.addSubview(&combo);
            }
            combo
        };
        let lang_combo = combo(200.0);
        let rate_combo = combo(170.0);
        let fs_check: Retained<NSButton> = unsafe {
            NSButton::checkboxWithTitle_target_action(
                &NSString::from_str("Fullscreen"),
                Some(super::app::anyobject(delegate)),
                None,
                mtm,
            )
        };
        fs_check.setFrame(NSRect::new(
            NSPoint::new(12.0, 140.0),
            NSSize::new(200.0, 22.0),
        ));
        if let Some(view) = window.contentView() {
            view.addSubview(&fs_check);
        }
        let err_label: Retained<NSTextField> = NSTextField::initWithFrame(
            mtm.alloc::<NSTextField>(),
            NSRect::new(NSPoint::new(12.0, 100.0), NSSize::new(380.0, 22.0)),
        );
        err_label.setEditable(false);
        err_label.setBezeled(false);
        err_label.setDrawsBackground(false);
        if let Some(view) = window.contentView() {
            view.addSubview(&err_label);
        }
        let button = |x: f64, title: &str, action: objc2::runtime::Sel| {
            let btn: Retained<NSButton> = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(title),
                    Some(super::app::anyobject(delegate)),
                    Some(action),
                    mtm,
                )
            };
            btn.setFrame(NSRect::new(NSPoint::new(x, 20.0), NSSize::new(90.0, 26.0)));
            if let Some(view) = window.contentView() {
                view.addSubview(&btn);
            }
        };
        button(200.0, "OK", sel!(submitSettings:));
        button(300.0, "Cancel", sel!(cancelSettings:));

        let gv = controller.vm.general.view.get();
        let av = controller.vm.audio.view.get();
        let vv = controller.vm.video.view.get();
        for choice in &gv.language_choices {
            unsafe {
                lang_combo.addItemWithObjectValue(&NSString::from_str(&choice.label));
            }
        }
        if let Some(i) = position(&gv.language_choices, &gv.language) {
            lang_combo.selectItemAtIndex(i as isize);
        }
        for choice in &av.sample_rate_choices {
            unsafe {
                rate_combo.addItemWithObjectValue(&NSString::from_str(&choice.label));
            }
        }
        if let Some(i) = position(&av.sample_rate_choices, &av.sample_rate) {
            rate_combo.selectItemAtIndex(i as isize);
        }
        if vv.fullscreen_default {
            fs_check.setState(NSControlStateValueOn);
        }
        window.makeKeyAndOrderFront(None);
        Some(Self {
            window,
            controller,
            lang_combo,
            rate_combo,
            fs_check,
            err_label,
            lang_choices: gv.language_choices,
            rate_choices: av.sample_rate_choices,
        })
    }

    fn selected<T: Clone + PartialEq + Eq>(
        combo: &NSComboBox,
        choices: &[ChoiceView<T>],
    ) -> Option<ChoiceView<T>> {
        let index = combo.indexOfSelectedItem();
        (index >= 0)
            .then(|| choices.get(index as usize).cloned())
            .flatten()
    }

    pub fn submit(&mut self, host: &mut nerust_mac::MacHost) -> bool {
        if let Some(choice) = Self::selected(&self.lang_combo, &self.lang_choices) {
            let _ = self.controller.update(SettingsEvent::SetLanguage(choice));
        }
        if let Some(choice) = Self::selected(&self.rate_combo, &self.rate_choices) {
            let _ = self.controller.update(SettingsEvent::SetSampleRate(choice));
        }
        let checked = self.fs_check.state() == NSControlStateValueOn;
        let _ = self
            .controller
            .update(SettingsEvent::ToggleFullscreenDefault(checked));
        let _ = self.controller.update(SettingsEvent::Submit);
        match host.close_settings(&self.controller) {
            Ok(true) => true,
            _ => {
                let banner = self.controller.validation_banner();
                self.err_label.setStringValue(&NSString::from_str(
                    banner.headline.as_deref().unwrap_or("invalid settings"),
                ));
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

fn position<T: Clone + PartialEq + Eq>(choices: &[ChoiceView<T>], value: &T) -> Option<usize> {
    choices.iter().position(|choice| &choice.value == value)
}

/// Headless verification hook mirroring the Win32 twin: submit the open
/// dialog through the mailbox protocol and close the sheet on success.
pub fn drive_submit() -> bool {
    let applied = super::app::with_settings(|dlg, host| dlg.submit(host)).unwrap_or(false);
    if applied {
        super::app::with_settings(|dlg, _| {
            dlg.close();
        });
    }
    applied
}
