//! Settings window: six pages bound to the shared `SettingsController`.
//!
//! A left `NavigationView` mirrors the tao sidebar (General/Input/Video/
//! Audio/Notifications/System); the content pane rebuilds from the
//! controller snapshot after every event, like the gtk twin's observer
//! refresh. OK stages through the mailbox protocol; Cancel discards.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use nerust_frontend_host::settings::{SettingsController, SettingsEvent};
use nerust_gui_viewmodel::settings::dto::ChoiceView;
use windows_core::{Interface, Result};

use crate::bindings::Microsoft::UI::Xaml::Controls::{
    Button, ComboBox, Grid, NavigationView, NavigationViewItem, Orientation,
    SelectionChangedEventHandler, StackPanel, TextBlock, TextBox, ToggleSwitch,
    UIElementCollection,
};
use crate::bindings::Microsoft::UI::Xaml::{RoutedEventHandler, Window};
use crate::bindings::Windows::Storage::Pickers::FolderPicker;

use super::{box_str, h};

pub struct SettingsWin {
    pub window: Window,
    controller: SettingsController,
    page_host: Grid,
    banner: TextBlock,
    ok_btn: Button,
    building: Arc<AtomicBool>,
    page: usize,
}

const PAGES: [&str; 6] = [
    "General",
    "Input",
    "Video",
    "Audio",
    "Notifications",
    "System",
];

impl SettingsWin {
    pub fn open(host: &mut nerust_win::WinHost) -> Option<Self> {
        let controller = host.open_settings();
        let flow = (|| -> Result<Self> {
            let window = Window::new()?;
            window.SetTitle(&h("Settings"))?;
            let nav = NavigationView::new()?;
            nav.SetPaneDisplayMode(
                crate::bindings::Microsoft::UI::Xaml::Controls::NavigationViewPaneDisplayMode::Left,
            )?;
            for (index, name) in PAGES.iter().enumerate() {
                let item = NavigationViewItem::new()?;
                item.SetContent(&box_str(name)?)?;
                let page = index;
                item.Tapped(
                    &crate::bindings::Microsoft::UI::Xaml::Input::TappedEventHandler::new(
                        move |_, _| {
                            super::app::with_app(|app| app.select_settings_page(page));
                            Ok(())
                        },
                    ),
                )?;
                nav.MenuItems()?.Append(&item)?;
            }
            let banner = TextBlock::new()?;
            let ok_btn = Button::new()?;
            ok_btn.SetContent(&box_str("OK")?)?;
            let cancel_btn = Button::new()?;
            cancel_btn.SetContent(&box_str("Cancel")?)?;
            ok_btn.Click(&RoutedEventHandler::new(|_, _| {
                super::app::with_app(|app| app.submit_settings());
                Ok(())
            }))?;
            cancel_btn.Click(&RoutedEventHandler::new(|_, _| {
                super::app::with_app(|app| app.cancel_settings());
                Ok(())
            }))?;
            let page_host = Grid::new()?;
            let buttons = StackPanel::new()?;
            buttons.SetOrientation(Orientation::Horizontal)?;
            buttons.Children()?.Append(&ok_btn)?;
            buttons.Children()?.Append(&cancel_btn)?;
            let content = StackPanel::new()?;
            content.SetOrientation(Orientation::Vertical)?;
            content.Children()?.Append(&page_host)?;
            content.Children()?.Append(&banner)?;
            content.Children()?.Append(&buttons)?;
            nav.SetContent(&content)?;
            window.SetContent(&nav)?;
            window.Activate()?;
            let mut win = Self {
                window,
                controller,
                page_host,
                banner,
                ok_btn,
                building: Arc::new(AtomicBool::new(false)),
                page: 0,
            };
            win.rebuild_page()?;
            Ok(win)
        })();
        match flow {
            Ok(win) => Some(win),
            Err(error) => {
                log::error!("winui settings open failed: {error}");
                None
            }
        }
    }

    pub fn controller_mut(&mut self) -> &mut SettingsController {
        &mut self.controller
    }

    pub fn set_page(&mut self, page: usize) {
        self.page = page.min(PAGES.len() - 1);
    }

    pub fn is_capturing(&self) -> bool {
        self.controller.vm.capture.view.get().target.is_some()
    }

    /// Full page rebuild from the controller snapshot.
    pub fn rebuild_page(&mut self) -> Result<()> {
        self.building.store(true, Ordering::SeqCst);
        let result = self.build_page();
        self.building.store(false, Ordering::SeqCst);
        result?;
        self.refresh_chrome();
        Ok(())
    }

    fn build_page(&mut self) -> Result<()> {
        self.page_host.Children()?.Clear()?;
        match self.page {
            0 => self.build_general(),
            1 => self.build_input(),
            2 => self.build_video(),
            3 => self.build_audio(),
            4 => self.build_notifications(),
            _ => self.build_system(),
        }
    }

    pub(crate) fn refresh_chrome(&mut self) {
        let banner = self.controller.validation_banner();
        let _ = self
            .banner
            .SetText(&h(banner.headline.as_deref().unwrap_or("")));
        let _ = self.ok_btn.SetIsEnabled(banner.can_submit);
    }

    fn section(&self, title: &str) -> Result<TextBlock> {
        let label = TextBlock::new()?;
        label.SetText(&h(title))?;
        Ok(label)
    }

    fn combo<T: Clone + PartialEq + Eq>(
        &self,
        choices: &[ChoiceView<T>],
        current: &T,
    ) -> Result<ComboBox> {
        let combo = ComboBox::new()?;
        let items = combo.Items()?;
        for choice in choices {
            items.Append(&box_str(&choice.label)?)?;
        }
        if let Some(index) = choices.iter().position(|choice| &choice.value == current) {
            combo.SetSelectedIndex(index as i32)?;
        }
        Ok(combo)
    }

    fn on_combo<T: Clone + PartialEq + Eq + Send + 'static>(
        &mut self,
        combo: &ComboBox,
        choices: Vec<ChoiceView<T>>,
        apply: impl Fn(ChoiceView<T>) -> SettingsEvent + Send + 'static,
    ) -> Result<()> {
        let building = self.building.clone();
        combo.SelectionChanged(&SelectionChangedEventHandler::new(move |sender, _| {
            if building.load(Ordering::SeqCst) {
                return Ok(());
            }
            let combo: ComboBox = sender
                .as_ref()
                .and_then(|sender| sender.cast().ok())
                .ok_or_else(|| {
                    windows_core::Error::new(
                        windows_core::HRESULT(0x80004005u32 as i32),
                        "combo cast",
                    )
                })?;
            let index = combo.SelectedIndex()?;
            if let Some(choice) = choices.get(index as usize).cloned() {
                super::app::with_app(|app| app.update_settings(apply(choice)));
            }
            Ok(())
        }))?;
        Ok(())
    }

    fn check(&self, label: &str, checked: bool) -> Result<ToggleSwitch> {
        let check = ToggleSwitch::new()?;
        check.SetHeader(&box_str(label)?)?;
        check.SetIsOn(checked)?;
        Ok(check)
    }

    fn on_check(
        &mut self,
        check: &ToggleSwitch,
        apply: impl Fn(bool) -> SettingsEvent + Send + 'static,
    ) -> Result<()> {
        let building = self.building.clone();
        check.Toggled(&RoutedEventHandler::new(move |sender, _| {
            if building.load(Ordering::SeqCst) {
                return Ok(());
            }
            let check: ToggleSwitch = sender
                .as_ref()
                .and_then(|sender| sender.cast().ok())
                .ok_or_else(|| {
                    windows_core::Error::new(
                        windows_core::HRESULT(0x80004005u32 as i32),
                        "check cast",
                    )
                })?;
            let on = check.IsOn().unwrap_or(false);
            super::app::with_app(|app| app.update_settings(apply(on)));
            Ok(())
        }))?;
        Ok(())
    }

    fn build_general(&mut self) -> Result<()> {
        let view = self.controller.vm.general.view.get();
        let stack = StackPanel::new()?;
        stack.SetOrientation(Orientation::Vertical)?;
        let items = stack.Children()?;
        items.Append(&self.section("Language")?)?;
        let combo = self.combo(&view.language_choices, &view.language)?;
        let choices = view.language_choices.clone();
        self.on_combo(&combo, choices, SettingsEvent::SetLanguage)?;
        items.Append(&combo)?;
        items.Append(&self.section("Save storage")?)?;
        let combo = self.combo(&view.storage_policy_choices, &view.storage_policy)?;
        let choices = view.storage_policy_choices.clone();
        self.on_combo(&combo, choices, SettingsEvent::SetStoragePolicy)?;
        items.Append(&combo)?;
        if view.show_storage_directory {
            let dir = TextBox::new()?;
            dir.SetText(&h(&view.storage_directory))?;
            dir.SetPlaceholderText(&h("Custom directory"))?;
            dir.TextChanged(
                &crate::bindings::Microsoft::UI::Xaml::Controls::TextChangedEventHandler::new(
                    |sender, _| {
                        let text: TextBox = sender
                            .as_ref()
                            .and_then(|sender| sender.cast().ok())
                            .ok_or_else(|| {
                            windows_core::Error::new(
                                windows_core::HRESULT(0x80004005u32 as i32),
                                "dir cast",
                            )
                        })?;
                        let value = text.Text()?.to_string();
                        super::app::with_app(|app| {
                            app.update_settings_no_rebuild(SettingsEvent::SetStorageDirectory(
                                value,
                            ))
                        });
                        Ok(())
                    },
                ),
            )?;
            items.Append(&dir)?;
            let browse = Button::new()?;
            browse.SetContent(&box_str("Browse...")?)?;
            browse.Click(&RoutedEventHandler::new(|_, _| {
                super::app::with_app(|app| app.browse_storage_directory());
                Ok(())
            }))?;
            items.Append(&browse)?;
            if let Some(error) = self.controller.storage_error() {
                let label = TextBlock::new()?;
                label.SetText(&h(&error))?;
                items.Append(&label)?;
            }
        }
        self.page_host.Children()?.Append(&stack)?;
        Ok(())
    }

    fn build_video(&mut self) -> Result<()> {
        let view = self.controller.vm.video.view.get();
        let stack = StackPanel::new()?;
        stack.SetOrientation(Orientation::Vertical)?;
        let items = stack.Children()?;
        let check = self.check("Fullscreen at startup", view.fullscreen_default)?;
        self.on_check(&check, SettingsEvent::ToggleFullscreenDefault)?;
        items.Append(&check)?;
        items.Append(&self.section("Scaling")?)?;
        let combo = self.combo(&view.scaling_choices, &view.scaling)?;
        let choices = view.scaling_choices.clone();
        self.on_combo(&combo, choices, SettingsEvent::SetScaling)?;
        items.Append(&combo)?;
        let check = self.check("VSync", view.vsync)?;
        self.on_check(&check, SettingsEvent::ToggleVsync)?;
        items.Append(&check)?;
        self.page_host.Children()?.Append(&stack)?;
        Ok(())
    }

    fn build_audio(&mut self) -> Result<()> {
        let view = self.controller.vm.audio.view.get();
        let stack = StackPanel::new()?;
        stack.SetOrientation(Orientation::Vertical)?;
        let items = stack.Children()?;
        let check = self.check("Mute", view.muted)?;
        self.on_check(&check, SettingsEvent::ToggleMute)?;
        items.Append(&check)?;
        items.Append(&self.section(&format!("Volume: {}%", view.volume_percent))?)?;
        let volume = TextBox::new()?;
        volume.SetText(&h(&view.volume_percent.to_string()))?;
        volume.SetPlaceholderText(&h("0-100"))?;
        volume.TextChanged(
            &crate::bindings::Microsoft::UI::Xaml::Controls::TextChangedEventHandler::new(
                |sender, _| {
                    use windows_core::Interface;
                    let text: TextBox = sender
                        .as_ref()
                        .and_then(|sender| sender.cast().ok())
                        .ok_or_else(|| {
                            windows_core::Error::new(
                                windows_core::HRESULT(0x80004005u32 as i32),
                                "volume cast",
                            )
                        })?;
                    if let Ok(value) = text.Text()?.to_string().parse::<u8>() {
                        super::app::with_app(|app| {
                            app.update_settings_no_rebuild(SettingsEvent::SetVolume(value))
                        });
                    }
                    Ok(())
                },
            ),
        )?;
        items.Append(&volume)?;
        items.Append(&self.section("Sample rate")?)?;
        let combo = self.combo(&view.sample_rate_choices, &view.sample_rate)?;
        let choices = view.sample_rate_choices.clone();
        self.on_combo(&combo, choices, SettingsEvent::SetSampleRate)?;
        items.Append(&combo)?;
        items.Append(&self.section(&format!("Latency: {} ms", view.latency_ms))?)?;
        let latency = TextBox::new()?;
        latency.SetText(&h(&view.latency_ms.to_string()))?;
        latency.TextChanged(
            &crate::bindings::Microsoft::UI::Xaml::Controls::TextChangedEventHandler::new(
                |sender, _| {
                    let text: TextBox = sender
                        .as_ref()
                        .and_then(|sender| sender.cast().ok())
                        .ok_or_else(|| {
                            windows_core::Error::new(
                                windows_core::HRESULT(0x80004005u32 as i32),
                                "latency cast",
                            )
                        })?;
                    if let Ok(value) = text.Text()?.to_string().parse::<u16>() {
                        super::app::with_app(|app| {
                            app.update_settings_no_rebuild(SettingsEvent::SetLatency(value))
                        });
                    }
                    Ok(())
                },
            ),
        )?;
        items.Append(&latency)?;
        self.page_host.Children()?.Append(&stack)?;
        Ok(())
    }

    fn build_notifications(&mut self) -> Result<()> {
        let view = self.controller.vm.notifications.view.get();
        let stack = StackPanel::new()?;
        stack.SetOrientation(Orientation::Vertical)?;
        let items = stack.Children()?;
        for (title, enabled, duration, color, on_enabled, on_duration, on_color) in [
            (
                "Info",
                view.info_enabled,
                view.info_duration_ms,
                view.info_color,
                SettingsEvent::ToggleInfoEnabled as fn(bool) -> SettingsEvent,
                SettingsEvent::SetInfoDuration as fn(u32) -> SettingsEvent,
                SettingsEvent::SetInfoColor
                    as fn(
                        ChoiceView<nerust_settings_core::model::local::NotificationColor>,
                    ) -> SettingsEvent,
            ),
            (
                "Warning",
                view.warning_enabled,
                view.warning_duration_ms,
                view.warning_color,
                SettingsEvent::ToggleWarningEnabled as fn(bool) -> SettingsEvent,
                SettingsEvent::SetWarningDuration as fn(u32) -> SettingsEvent,
                SettingsEvent::SetWarningColor
                    as fn(
                        ChoiceView<nerust_settings_core::model::local::NotificationColor>,
                    ) -> SettingsEvent,
            ),
        ] {
            items.Append(&self.section(title)?)?;
            let check = self.check("Enabled", enabled)?;
            self.on_check(&check, on_enabled)?;
            items.Append(&check)?;
            let dur_box = TextBox::new()?;
            dur_box.SetText(&h(&duration.to_string()))?;
            dur_box.TextChanged(
                &crate::bindings::Microsoft::UI::Xaml::Controls::TextChangedEventHandler::new(
                    move |sender, _| {
                        let text: TextBox = sender
                            .as_ref()
                            .and_then(|sender| sender.cast().ok())
                            .ok_or_else(|| {
                            windows_core::Error::new(
                                windows_core::HRESULT(0x80004005u32 as i32),
                                "duration cast",
                            )
                        })?;
                        if let Ok(value) = text.Text()?.to_string().parse::<u32>() {
                            super::app::with_app(|app| {
                                app.update_settings_no_rebuild(on_duration(value))
                            });
                        }
                        Ok(())
                    },
                ),
            )?;
            items.Append(&dur_box)?;
            let combo = self.combo(&view.color_choices, &color)?;
            let choices = view.color_choices.clone();
            self.on_combo(&combo, choices, on_color)?;
            items.Append(&combo)?;
        }
        items.Append(&self.section("Error")?)?;
        let check = self.check("Enabled", view.error_enabled)?;
        self.on_check(&check, SettingsEvent::ToggleErrorEnabled)?;
        items.Append(&check)?;
        items.Append(&self.section("Error display")?)?;
        let display = self.combo(&[modal_choice(), overlay_choice()], &view.error_modal)?;
        self.on_combo(&display, vec![modal_choice(), overlay_choice()], |choice| {
            SettingsEvent::ToggleErrorModal(choice.value)
        })?;
        items.Append(&display)?;
        let combo = self.combo(&view.color_choices, &view.error_color)?;
        let choices = view.color_choices.clone();
        self.on_combo(&combo, choices, SettingsEvent::SetErrorColor)?;
        items.Append(&combo)?;
        items.Append(&self.section("Position")?)?;
        let combo = self.combo(&view.position_choices, &view.position)?;
        let choices = view.position_choices.clone();
        self.on_combo(&combo, choices, SettingsEvent::SetPosition)?;
        items.Append(&combo)?;
        items.Append(&self.section("Area mode")?)?;
        let combo = self.combo(&view.area_mode_choices, &view.area_mode)?;
        let choices = view.area_mode_choices.clone();
        self.on_combo(&combo, choices, SettingsEvent::SetAreaMode)?;
        items.Append(&combo)?;
        let check = self.check("Status line enabled", view.status_enabled)?;
        self.on_check(&check, SettingsEvent::ToggleStatusEnabled)?;
        items.Append(&check)?;
        items.Append(&self.section("Status position")?)?;
        let combo = self.combo(&view.position_choices, &view.status_position)?;
        let choices = view.position_choices.clone();
        self.on_combo(&combo, choices, SettingsEvent::SetStatusPosition)?;
        items.Append(&combo)?;
        items.Append(&self.section("Status area mode")?)?;
        let combo = self.combo(&view.area_mode_choices, &view.status_area_mode)?;
        let choices = view.area_mode_choices.clone();
        self.on_combo(&combo, choices, SettingsEvent::SetStatusAreaMode)?;
        items.Append(&combo)?;
        self.page_host.Children()?.Append(&stack)?;
        Ok(())
    }

    fn build_input(&mut self) -> Result<()> {
        let stack = StackPanel::new()?;
        stack.SetOrientation(Orientation::Vertical)?;
        let items = stack.Children()?;
        let inputs: Vec<_> = self
            .controller
            .vm
            .inputs()
            .iter()
            .map(|tab| tab.view.get())
            .collect();
        for view in &inputs {
            items.Append(&self.section(&view.label)?)?;
            for slot in &view.slots {
                let row = StackPanel::new()?;
                row.SetOrientation(Orientation::Horizontal)?;
                let row_items = row.Children()?;
                let label = TextBlock::new()?;
                label.SetText(&h(&slot.label))?;
                row_items.Append(&label)?;
                let combo = self.combo(&slot.choices, &slot.selected_profile_id)?;
                let choices = slot.choices.clone();
                let slot_id = slot.slot_id;
                let building = self.building.clone();
                combo.SelectionChanged(&SelectionChangedEventHandler::new(move |sender, _| {
                    if building.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                    let combo: ComboBox = sender
                        .as_ref()
                        .and_then(|sender| sender.cast().ok())
                        .ok_or_else(|| {
                            windows_core::Error::new(
                                windows_core::HRESULT(0x80004005u32 as i32),
                                "slot cast",
                            )
                        })?;
                    let index = combo.SelectedIndex()?;
                    if let Some(choice) = choices.get(index as usize).cloned() {
                        super::app::with_app(|app| {
                            app.update_settings(SettingsEvent::SetControllerSlot {
                                slot: slot_id,
                                controller_id: choice.value,
                            })
                        });
                    }
                    Ok(())
                }))?;
                row_items.Append(&combo)?;
                if slot.occupied_by_other_slot {
                    let taken = TextBlock::new()?;
                    taken.SetText(&h("(occupied)"))?;
                    row_items.Append(&taken)?;
                }
                items.Append(&row)?;
            }
            for conflict in &view.conflicts {
                let label = TextBlock::new()?;
                label.SetText(&h(&conflict.message))?;
                items.Append(&label)?;
            }
            for section in &view.sections {
                items.Append(&self.section(&section.label)?)?;
                for row in &section.rows {
                    self.append_binding_row(
                        &items,
                        &row.label,
                        &row_value(row),
                        row.target.clone(),
                    )?;
                }
            }
        }
        items.Append(&self.section("Shortcuts")?)?;
        let shortcuts = self.controller.vm.shortcuts.view.get();
        for row in &shortcuts.rows {
            self.append_binding_row(&items, &row.label, &row_value(row), row.target.clone())?;
        }
        let capture = self.controller.vm.capture.view.get();
        let prompt = TextBlock::new()?;
        prompt.SetText(&h(&capture.prompt))?;
        items.Append(&prompt)?;
        self.page_host.Children()?.Append(&stack)?;
        Ok(())
    }

    fn append_binding_row(
        &mut self,
        items: &UIElementCollection,
        label: &str,
        value: &str,
        target: nerust_settings_core::editor::CaptureTarget,
    ) -> Result<()> {
        let row = StackPanel::new()?;
        row.SetOrientation(Orientation::Horizontal)?;
        let row_items = row.Children()?;
        let name = TextBlock::new()?;
        name.SetText(&h(label))?;
        row_items.Append(&name)?;
        let current = TextBlock::new()?;
        current.SetText(&h(value))?;
        row_items.Append(&current)?;
        let change = Button::new()?;
        change.SetContent(&box_str("Change")?)?;
        let t1 = target.clone();
        let t2 = target.clone();
        change.Click(&RoutedEventHandler::new(move |_, _| {
            super::app::with_app(|app| {
                app.update_settings(SettingsEvent::StartCapture(t1.clone()))
            });
            Ok(())
        }))?;
        row_items.Append(&change)?;
        let clear = Button::new()?;
        clear.SetContent(&box_str("Clear")?)?;
        clear.Click(&RoutedEventHandler::new(move |_, _| {
            super::app::with_app(|app| {
                app.update_settings(SettingsEvent::ClearCapture(t2.clone()))
            });
            Ok(())
        }))?;
        row_items.Append(&clear)?;
        items.Append(&row)?;
        Ok(())
    }

    fn build_system(&mut self) -> Result<()> {
        let stack = StackPanel::new()?;
        stack.SetOrientation(Orientation::Vertical)?;
        let items = stack.Children()?;
        let systems: Vec<_> = self
            .controller
            .vm
            .systems()
            .iter()
            .map(|tab| tab.view.get())
            .collect();
        for view in &systems {
            items.Append(&self.section(&view.label)?)?;
            for field in &view.fields {
                let label = TextBlock::new()?;
                label.SetText(&h(&field.label))?;
                items.Append(&label)?;
                let combo = self.combo(&field.choices, &field.selected)?;
                let choices = field.choices.clone();
                let field_id = field.id.clone();
                let building = self.building.clone();
                combo.SelectionChanged(&SelectionChangedEventHandler::new(move |sender, _| {
                    if building.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                    let combo: ComboBox = sender
                        .as_ref()
                        .and_then(|sender| sender.cast().ok())
                        .ok_or_else(|| {
                            windows_core::Error::new(
                                windows_core::HRESULT(0x80004005u32 as i32),
                                "field cast",
                            )
                        })?;
                    let index = combo.SelectedIndex()?;
                    if let Some(choice) = choices.get(index as usize).cloned() {
                        let id = field_id.0.clone().into_owned();
                        super::app::with_app(|app| {
                            app.update_settings(SettingsEvent::SetSystemChoice(id, choice))
                        });
                    }
                    Ok(())
                }))?;
                items.Append(&combo)?;
            }
        }
        self.page_host.Children()?.Append(&stack)?;
        Ok(())
    }

    pub fn close(&self) {
        let _ = self.window.Close();
    }

    pub fn submit(&mut self, host: &mut nerust_win::WinHost) -> bool {
        let _ = self.controller.update(SettingsEvent::Submit);
        match host.close_settings(&self.controller) {
            Ok(true) => true,
            _ => {
                self.refresh_chrome();
                false
            }
        }
    }

    pub fn cancel(&mut self, host: &mut nerust_win::WinHost) {
        let _ = host.close_settings(&self.controller);
    }

    pub fn browse_storage_directory(&mut self) {
        let picker = match FolderPicker::new() {
            Ok(picker) => picker,
            Err(error) => {
                log::error!("winui folder picker failed: {error}");
                return;
            }
        };
        let flow = (|| -> Result<()> {
            use crate::bindings::Windows::Win32::Foundation::HWND;
            use crate::bindings::Windows::Win32::UI::Shell::IInitializeWithWindow;
            let hwnd = super::window_handle(&self.window)?;
            let init: IInitializeWithWindow = picker.cast()?;
            unsafe { init.Initialize(HWND(hwnd as *mut std::ffi::c_void))? };
            picker.FileTypeFilter()?.Append(&h("*"))?;
            let op = picker.PickSingleFolderAsync()?;
            let queue = self.window.DispatcherQueue()?;
            op.SetCompleted(&windows_future::AsyncOperationCompletedHandler::new(
                    move |op, _| {
                        use crate::bindings::Windows::Storage::StorageFolder;
                        use windows_future::IAsyncOperation;
                        let path = op
                            .as_ref()
                            .and_then(|op: &IAsyncOperation<StorageFolder>| op.GetResults().ok())
                            .and_then(|folder: StorageFolder| folder.Path().ok())
                            .map(|path| path.to_string());
                        if let Some(path) = path {
                            let queue = queue.clone();
                            let dir = std::path::PathBuf::from(path);
                            queue.TryEnqueue(
                                &crate::bindings::Microsoft::UI::Dispatching::DispatcherQueueHandler::new(
                                    move || {
                                        super::app::with_app(|app| {
                                            app.update_settings(
                                                SettingsEvent::StorageDirectorySelected(Some(
                                                    dir.clone(),
                                                )),
                                            )
                                        });
                                        Ok(())
                                    },
                                ),
                            )?;
                        }
                        Ok(())
                    },
                ),
            )?;
            Ok(())
        })();
        if let Err(error) = flow {
            log::error!("winui folder picker failed: {error}");
        }
    }
}

fn row_value(row: &nerust_gui_viewmodel::settings::dto::BindingRowView) -> String {
    use nerust_gui_viewmodel::settings::dto::BindingValueView;
    match &row.value {
        BindingValueView::Unbound(text)
        | BindingValueView::Bound(text)
        | BindingValueView::Capturing(text) => text.clone(),
    }
}

fn modal_choice() -> ChoiceView<bool> {
    ChoiceView {
        label: "Modal dialog".to_string(),
        value: true,
    }
}

fn overlay_choice() -> ChoiceView<bool> {
    ChoiceView {
        label: "On-screen".to_string(),
        value: false,
    }
}
