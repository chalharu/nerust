use super::retained::{NativeControl, NativeKind, NodeId};
use super::*;
use nerust_frontend_host::settings::{
    InputPageSection, InputTab, SettingsControlKind, SettingsPage, SettingsTabsId,
};
use nerust_gui_viewmodel::settings::dto::ChoiceView;
use nerust_settings_core::model::{language::AppLanguage, shared::StoragePolicy};

struct Harness {
    binding: Rc<PreferencesBinding>,
    dialog: gtk::Dialog,
}

impl Harness {
    fn new(system: bool) -> Self {
        assert!(
            crate::test_support::ui_test_available(),
            "real GTK display required; run this test in its own xvfb-run process"
        );
        let mut snapshot = nerust_settings_core::model::snapshot::SettingsSnapshot {
            shared: Default::default(),
            local: Default::default(),
            app_state: Default::default(),
        };
        snapshot.shared.general.language = AppLanguage::English;
        nerust_settings_core::model::input::ensure_default_shortcuts(
            &mut snapshot.shared.input.shortcuts,
        );
        let factories = if system {
            vec![nerust_nes_factory::NesFactory::boxed().into()]
        } else {
            vec![]
        };
        let controller = SettingsController::new(
            &snapshot,
            Arc::new(nerust_gui_shell::registry::SystemRegistry::new(factories)),
            Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new()),
        );
        let dialog = gtk::Dialog::builder()
            .default_width(900)
            .default_height(560)
            .build();
        let binding = PreferencesBinding::new(controller, &dialog);
        Self { binding, dialog }
    }

    fn control(&self, id: &SettingsControlId) -> Rc<NativeControl> {
        self.binding
            .registry
            .borrow()
            .control(id)
            .expect("retained control")
    }

    fn custom_directory(&self) {
        self.binding
            .send([SettingsEvent::SetStoragePolicy(ChoiceView {
                value: StoragePolicy::CustomDirectory,
                label: String::new(),
            })]);
    }

    fn notebook(&self, id: SettingsTabsId) -> gtk::Notebook {
        self.binding
            .registry
            .borrow()
            .node(&NodeId::Tabs(id))
            .unwrap()
            .widget
            .downcast()
            .unwrap()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.binding.close();
        self.dialog.destroy();
    }
}

fn pump() {
    let context = glib::MainContext::default();
    for _ in 0..2000 {
        if !context.pending() {
            return;
        }
        context.iteration(false);
    }
    panic!("main context did not become idle");
}

fn assert_accessible_label(widget: &gtk::Widget, expected: &str) {
    use glib::translate::{IntoGlib, ToGlibPtr};

    // gtktestatcontext.h marks this GTK 4.0 API GDK_AVAILABLE_IN_ALL.
    // As in retained.rs, use the same ABI with void* because gtk-rs gates
    // GtkAccessible behind v4_10; no additional Cargo feature is needed.
    unsafe extern "C" {
        fn gtk_test_accessible_check_property(
            accessible: *mut std::ffi::c_void,
            property: gtk::ffi::GtkAccessibleProperty,
            ...
        ) -> *mut std::ffi::c_char;
    }
    let expected = expected.to_value();
    // SAFETY: Widget implements Accessible. Label expects a C string, NOT a
    // GValue pointer. g_value_get_string borrows the live string GValue for
    // this synchronous variadic call. NULL means a match (see the header's
    // gtk_test_accessible_assert_property macro). Gtk-4.0.gir declares the
    // mismatch string transfer-ownership="full": copy it, then g_free it,
    // including on the assertion-failure path.
    let mismatch = unsafe {
        let actual = gtk_test_accessible_check_property(
            widget.as_ptr().cast(),
            gtk::AccessibleProperty::Label.into_glib(),
            glib::gobject_ffi::g_value_get_string(expected.to_glib_none().0),
        );
        if actual.is_null() {
            None
        } else {
            let message = std::ffi::CStr::from_ptr(actual)
                .to_string_lossy()
                .into_owned();
            glib::ffi::g_free(actual.cast());
            Some(message)
        }
    };
    assert!(
        mismatch.is_none(),
        "GTK accessible Label: expected {:?}, actual {mismatch:?}",
        expected.get::<String>().unwrap()
    );
}

fn assert_note_name(h: &Harness, id: &SettingsControlId, labelled: bool) -> String {
    let screen = h.binding.controller.borrow().screen();
    let control = screen.control(id).expect("host note");
    let SettingsControlKind::Note { text } = &control.kind else {
        panic!("host note kind")
    };
    assert!(
        !text.is_empty(),
        "exercise a real note, not an empty placeholder"
    );
    assert_eq!(!control.label.is_empty(), labelled);
    let expected = if labelled {
        format!("{}: {text}", control.label)
    } else {
        text.clone()
    };
    let native = h.control(id);
    let NativeKind::Note(label) = &native.kind else {
        panic!("native note kind")
    };
    assert_eq!(label.text().as_str(), text);
    assert!(native.root.property::<bool>("visible"));
    assert!(native.input.property::<bool>("visible"));
    // Inspect GTK itself, not just the host DTO or the visible label text.
    assert_accessible_label(&native.input, &expected);
    assert_accessible_label(&native.root, &expected);
    assert_eq!(control.metadata.accessible_label, expected);
    expected
}

fn exercise_note_refreshes(h: &Harness, id: SettingsControlId, labelled: bool) {
    let native = h.control(&id);
    let english = assert_note_name(h, &id, labelled);
    h.binding.send([SettingsEvent::ToggleMute(true)]);
    assert_eq!(assert_note_name(h, &id, labelled), english);
    for _ in 0..3 {
        h.binding.refresh();
        pump();
        assert_eq!(assert_note_name(h, &id, labelled), english);
    }
    h.binding.send([SettingsEvent::SetLanguage(ChoiceView {
        value: AppLanguage::Japanese,
        label: String::new(),
    })]);
    let japanese = assert_note_name(h, &id, labelled);
    assert_eq!(
        h.binding.controller.borrow().screen().language,
        AppLanguage::Japanese
    );
    // Storage validation currently exposes a language-independent diagnostic.
    // The other notes must actually relabel, not merely survive a no-op locale.
    if id != SettingsControlId::StorageError {
        assert_ne!(
            japanese, english,
            "translation must actually change the note"
        );
    }
    h.binding.send([SettingsEvent::ToggleMute(false)]);
    for _ in 0..3 {
        h.binding.refresh();
        pump();
        assert_eq!(assert_note_name(h, &id, labelled), japanese);
    }
    assert!(Rc::ptr_eq(&native, &h.control(&id)));
}

#[test]
fn retained_no_systems_accessible_name_survives_refresh_and_translation() {
    let h = Harness::new(false);
    exercise_note_refreshes(&h, SettingsControlId::NoSystems, false);
}

#[test]
fn retained_storage_error_accessible_name_survives_refresh_and_translation() {
    let h = Harness::new(false);
    h.custom_directory();
    h.binding
        .send([SettingsEvent::SetStorageDirectory(String::new())]);
    exercise_note_refreshes(&h, SettingsControlId::StorageError, false);
}

#[test]
fn retained_labelled_note_accessible_name_includes_value_after_refresh_and_translation() {
    let h = Harness::new(false);
    h.binding
        .send([SettingsEvent::SetStatusPosition(ChoiceView {
            value: nerust_settings_core::model::local::NotificationPosition::BottomLeft,
            label: String::new(),
        })]);
    exercise_note_refreshes(&h, SettingsControlId::StatusAreaFollows, true);
}

#[test]
fn retained_all_pages_metadata_ranges_and_values_follow_host() {
    let h = Harness::new(true);
    let screen = h.binding.controller.borrow().screen();
    assert_eq!(screen.pages.len(), 6);
    assert_eq!(h.dialog.title().as_deref(), Some(screen.title.as_str()));
    let controls = h.binding.registry.borrow().controls.clone();
    for (id, native) in controls {
        let control = screen.control(&id).unwrap();
        assert_eq!(
            native.root.property::<bool>("visible"),
            control.metadata.visible,
            "{id:?}"
        );
        assert_eq!(
            native.input.is_sensitive(),
            control.metadata.enabled,
            "{id:?}"
        );
        match (&native.kind, &control.kind) {
            (NativeKind::Toggle(check), SettingsControlKind::Toggle { value }) => {
                assert_eq!(check.is_active(), *value)
            }
            (NativeKind::Choice(combo, _), SettingsControlKind::Choice { selected, .. }) => {
                assert_eq!(combo.active(), selected.map(|i| i as u32))
            }
            (
                NativeKind::Number(spin, label),
                SettingsControlKind::Number {
                    value,
                    min,
                    max,
                    step,
                    unit,
                    ..
                },
            ) => {
                assert_eq!(spin.value(), f64::from(*value));
                assert_eq!(spin.range(), (f64::from(*min), f64::from(*max)));
                assert_eq!(spin.increments().0, f64::from(*step));
                assert_eq!(label.text().as_str(), unit);
            }
            _ => (),
        }
    }
    for page in &screen.pages {
        let node = h
            .binding
            .registry
            .borrow()
            .node(&NodeId::Page(page.id))
            .unwrap();
        assert_eq!(
            h.binding.stack.page(&node.widget).title().as_deref(),
            Some(page.label.as_str())
        );
    }
}

#[test]
fn retained_entry_preserves_identity_cursor_selection_focus_and_single_edit() {
    let h = Harness::new(false);
    h.custom_directory();
    let native = h.control(&SettingsControlId::StorageDirectory);
    let NativeKind::Text(entry, _) = &native.kind else {
        panic!("text")
    };
    entry.set_text("/tmp/unfinished-directory");
    h.dialog.present();
    pump();
    assert!(entry.grab_focus());
    entry.select_region(2, 7);
    let selection = entry.selection_bounds();
    let position = entry.position();
    let focus = GtkWindowExt::focus(&h.dialog).expect("native text focus");
    let edits = Rc::new(Cell::new(0));
    entry.connect_changed({
        let edits = edits.clone();
        move |_| edits.set(edits.get() + 1)
    });
    h.binding.send([SettingsEvent::ToggleMute(true)]);
    h.binding.refresh();
    pump();
    assert!(Rc::ptr_eq(
        &native,
        &h.control(&SettingsControlId::StorageDirectory)
    ));
    assert_eq!(entry.selection_bounds(), selection);
    assert_eq!(entry.position(), position);
    assert_eq!(GtkWindowExt::focus(&h.dialog).as_ref(), Some(&focus));
    assert_eq!(edits.get(), 0, "unrelated refresh must not reset text/IME");
    entry.select_region(0, 0);
    entry.insert_text("x", &mut 3);
    assert_eq!(edits.get(), 1, "model feedback must not cause another edit");
    let screen = h.binding.controller.borrow().screen();
    let SettingsControlKind::Text { value, .. } = &screen
        .control(&SettingsControlId::StorageDirectory)
        .unwrap()
        .kind
    else {
        panic!("text")
    };
    assert_eq!(value, entry.text().as_str());
}

#[test]
fn retained_same_widgets_update_values_translation_and_validation() {
    let h = Harness::new(false);
    let mute = h.control(&SettingsControlId::Mute);
    let language = h.control(&SettingsControlId::Language);
    let NativeKind::Toggle(check) = &mute.kind else {
        panic!("toggle")
    };
    h.binding.send([SettingsEvent::ToggleMute(true)]);
    assert!(check.is_active());
    h.binding.send([SettingsEvent::SetLanguage(ChoiceView {
        value: AppLanguage::Japanese,
        label: String::new(),
    })]);
    let screen = h.binding.controller.borrow().screen();
    assert_eq!(
        check.label().as_deref(),
        Some(
            screen
                .control(&SettingsControlId::Mute)
                .unwrap()
                .label
                .as_str()
        )
    );
    assert_eq!(h.dialog.title().as_deref(), Some(screen.title.as_str()));
    assert!(Rc::ptr_eq(
        &language,
        &h.control(&SettingsControlId::Language)
    ));
    assert!(Rc::ptr_eq(&mute, &h.control(&SettingsControlId::Mute)));
    h.custom_directory();
    h.binding
        .send([SettingsEvent::SetStorageDirectory(String::new())]);
    let ok = h.control(&SettingsControlId::Submit);
    assert!(!ok.input.is_sensitive());
    assert!(!h.binding.error_label.text().is_empty());
    h.binding
        .input(&SettingsControlId::Submit, ControlInput::Activate);
    assert!(h.binding.controller.borrow().take_pending_apply().is_none());
    h.binding.send([SettingsEvent::SetStorageDirectory(
        std::env::temp_dir().to_string_lossy().into(),
    )]);
    assert!(ok.input.is_sensitive());
    assert!(h.binding.error_label.text().is_empty());
    h.binding
        .input(&SettingsControlId::Submit, ControlInput::Activate);
    assert!(h.binding.controller.borrow().take_pending_apply().is_some());
}

#[test]
fn retained_native_stack_notebooks_and_context_reset_mirror_model() {
    let h = Harness::new(true);
    h.dialog.present();
    let input_page = h
        .binding
        .registry
        .borrow()
        .node(&NodeId::Page(SettingsPage::Input))
        .unwrap()
        .widget;
    h.binding.stack.set_visible_child(&input_page);
    assert_eq!(h.binding.controller.borrow().page, SettingsPage::Input);
    let inputs = h.notebook(SettingsTabsId::Inputs);
    h.binding.send([SettingsEvent::SelectInputSection(
        InputPageSection::Attachment(1),
    )]);
    inputs.set_current_page(Some(inputs.n_pages() - 1));
    pump();
    assert_eq!(
        h.binding.controller.borrow().input_tab,
        Some(InputTab::Shortcuts)
    );
    assert_eq!(
        h.binding.controller.borrow().input_section,
        InputPageSection::Attachment(0)
    );
    inputs.set_current_page(Some(0));
    pump();
    assert_eq!(
        h.binding.controller.borrow().input_tab,
        Some(InputTab::System(0))
    );
    h.binding
        .send([SettingsEvent::SelectPage(SettingsPage::Audio)]);
    let audio_page = h
        .binding
        .registry
        .borrow()
        .node(&NodeId::Page(SettingsPage::Audio))
        .unwrap()
        .widget;
    assert_eq!(h.binding.stack.visible_child(), Some(audio_page));
}

#[test]
fn retained_binding_capture_key_clear_and_dynamic_topology() {
    let h = Harness::new(true);
    let (id, native) = h
        .binding
        .registry
        .borrow()
        .controls
        .iter()
        .find(|(id, _)| {
            matches!(
                id,
                SettingsControlId::Binding(nerust_settings_core::editor::CaptureTarget::Shortcut(
                    _
                ))
            )
        })
        .map(|(id, native)| (id.clone(), native.clone()))
        .unwrap();
    let NativeKind::Binding(value, change, clear) = &native.kind else {
        panic!("binding")
    };
    change.emit_clicked();
    assert!(
        h.binding
            .controller
            .borrow()
            .screen()
            .capture
            .target
            .is_some()
    );
    assert!(Rc::ptr_eq(&native, &h.control(&id)));
    let controllers = h.dialog.observe_controllers();
    let keys = (0..controllers.n_items())
        .find_map(|i| {
            controllers
                .item(i)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .unwrap();
    let handled = keys.emit_by_name::<bool>(
        "key-pressed",
        &[&gdk::Key::F2, &0u32, &gdk::ModifierType::empty()],
    );
    assert!(handled);
    assert!(
        h.binding
            .controller
            .borrow()
            .screen()
            .capture
            .target
            .is_none()
    );
    assert_eq!(value.text().as_str(), "F2");
    let SettingsControlId::Binding(target) = &id else {
        unreachable!()
    };
    assert_eq!(
        nerust_settings_core::editor::current_binding_key(
            &h.binding.controller.borrow().vm.snapshot(),
            target
        ),
        Some(nerust_keyboard::Key::F2)
    );
    clear.emit_clicked();
    assert!(matches!(
        h.binding
            .controller
            .borrow()
            .screen()
            .control(&id)
            .unwrap()
            .kind,
        SettingsControlKind::Binding {
            value: nerust_gui_viewmodel::settings::dto::BindingValueView::Unbound(_),
            ..
        }
    ));
    let before = h.binding.registry.borrow().controls.len();
    let (slot_id, slot) = h
        .binding
        .registry
        .borrow()
        .controls
        .iter()
        .find(|(id, _)| matches!(id, SettingsControlId::ControllerSlot { .. }))
        .map(|(id, native)| (id.clone(), native.clone()))
        .unwrap();
    let NativeKind::Choice(combo, _) = &slot.kind else {
        panic!("slot")
    };
    combo.set_active(Some(0));
    assert!(
        h.binding.registry.borrow().controls.len() < before,
        "removing attachment diffs binding rows"
    );
    assert!(Rc::ptr_eq(&slot, &h.control(&slot_id)));
    assert!(
        Rc::ptr_eq(&native, &h.control(&id)),
        "unrelated shortcuts survive topology change"
    );
}

#[test]
fn retained_browse_is_accessible_nonblocking_and_cancel_preserves_text() {
    let h = Harness::new(false);
    h.custom_directory();
    h.binding
        .send([SettingsEvent::SetStorageDirectory("/tmp".into())]);
    let native = h.control(&SettingsControlId::StorageDirectory);
    let NativeKind::Text(entry, browse) = &native.kind else {
        panic!("text")
    };
    assert!(browse.property::<bool>("visible"));
    assert!(!browse.label().unwrap().is_empty());
    assert!(
        h.binding
            .controller
            .borrow()
            .screen()
            .bind(&SettingsControlId::StorageDirectory, ControlInput::Activate)
            .is_some()
    );
    browse.emit_clicked();
    let chooser = h
        .binding
        .chooser
        .borrow()
        .clone()
        .expect("async native chooser returned without blocking");
    assert_eq!(chooser.action(), gtk::FileChooserAction::SelectFolder);
    chooser.emit_by_name::<()>(
        "response",
        &[&glib::translate::IntoGlib::into_glib(
            gtk::ResponseType::Cancel,
        )],
    );
    assert!(h.binding.chooser.borrow().is_none());
    assert_eq!(entry.text().as_str(), "/tmp");
    browse.emit_clicked();
    let next = h.binding.chooser.borrow().clone().unwrap();
    chooser.emit_by_name::<()>(
        "response",
        &[&glib::translate::IntoGlib::into_glib(
            gtk::ResponseType::Cancel,
        )],
    );
    assert_eq!(
        h.binding.chooser.borrow().as_ref(),
        Some(&next),
        "stale responses cannot clear the new chooser"
    );
    next.emit_by_name::<()>(
        "response",
        &[&glib::translate::IntoGlib::into_glib(
            gtk::ResponseType::Cancel,
        )],
    );
    h.binding
        .send([SettingsEvent::StorageDirectorySelected(Some(
            std::env::temp_dir(),
        ))]);
    assert_eq!(
        entry.text().as_str(),
        std::env::temp_dir().to_string_lossy()
    );
}

#[test]
fn retained_synchronous_setter_reentry_is_borrow_free_and_latest_wins() {
    let h = Harness::new(false);
    let mute = h.control(&SettingsControlId::Mute);
    let NativeKind::Toggle(check) = &mute.kind else {
        panic!("toggle")
    };
    let called = Rc::new(Cell::new(false));
    check.connect_toggled({
        let weak = Rc::downgrade(&h.binding);
        let called = called.clone();
        move |_| {
            let binding = weak.upgrade().unwrap();
            assert!(binding.controller.try_borrow_mut().is_ok());
            assert!(binding.registry.try_borrow_mut().is_ok());
            if !called.replace(true) {
                binding.send([SettingsEvent::ToggleMute(false)]);
                binding.refresh();
            }
        }
    });
    h.binding.send([SettingsEvent::ToggleMute(true)]);
    assert!(called.get());
    assert!(
        !check.is_active(),
        "newest reentrant screen replaces outer stale values"
    );
    assert!(matches!(
        h.binding
            .controller
            .borrow()
            .screen()
            .control(&SettingsControlId::Mute)
            .unwrap()
            .kind,
        SettingsControlKind::Toggle { value: false }
    ));
    assert!(!h.binding.refreshing.get());
    assert!(!h.binding.updating.get());
}

#[test]
fn retained_revision_observer_defers_and_weak_idle_does_not_retain_binding() {
    let h = Harness::new(false);
    let native = h.control(&SettingsControlId::Mute);
    let NativeKind::Toggle(check) = &native.kind else {
        panic!("toggle")
    };
    {
        let mut controller = h.binding.controller.borrow_mut();
        controller.update(SettingsEvent::ToggleMute(true));
        controller.update(SettingsEvent::SetVolume(23));
        assert!(h.binding.scheduled.get());
        assert!(
            !check.is_active(),
            "observer must not touch GTK within controller borrow"
        );
    }
    pump();
    assert!(check.is_active());
    assert!(!h.binding.scheduled.get());
    let weak = Rc::downgrade(&h.binding);
    h.binding.schedule();
    drop(h);
    assert!(weak.upgrade().is_none());
    pump();
}

#[test]
fn preferences_production_response_save_failure_edit_retry_closes_once() {
    use nerust_gui_runtime::settings::{
        HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
    };
    use nerust_gui_shell::{context::FrontendContext, session::SessionHandle};

    #[derive(Debug)]
    struct NoRenderer;
    impl nerust_render_traits::renderer::PipelineFactory for NoRenderer {
        fn create_pipeline(
            &self,
            _: &nerust_render_traits::renderer::RendererConfig,
            _: raw_window_handle::RawDisplayHandle,
        ) -> Result<
            Box<dyn nerust_render_traits::renderer::RenderPipeline>,
            nerust_render_traits::renderer::RendererError,
        > {
            panic!("preferences must not create a renderer")
        }
    }

    #[derive(Debug)]
    struct NoRomLoader;
    impl nerust_gui_shell::load::RomLoader for NoRomLoader {
        fn load_rom(
            &mut self,
            _: &std::path::Path,
            _: &mut dyn nerust_gui_shell::load::RomLoadTarget,
        ) -> Result<(), nerust_gui_shell::load::RomLoaderError> {
            panic!("preferences must not load a ROM")
        }
    }

    struct TempDirectory(std::path::PathBuf);
    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut widgets = vec![widget.clone()];
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            widgets.extend(descendants(&current));
        }
        widgets
    }

    assert!(
        crate::test_support::ui_test_available(),
        "real GTK display required; run in its own xvfb-run process"
    );
    let dir = TempDirectory(std::env::temp_dir().join(format!(
        "nerust-preferences-response-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    let paths = SettingsPaths {
        settings_file: dir.0.join("settings.yaml"),
        central_storage_root: dir.0.join("storage"),
    };
    let registry = Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![]));
    let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let capabilities = HostBackendCapabilities {
        window: HostWindowCapabilities {
            remembers_window_size: false,
            supports_fullscreen_default: true,
            supports_scaling: true,
        },
        presentation: None,
    };
    let session = SessionHandle::new_with_settings_paths(
        capabilities,
        registry.clone(),
        audio.clone(),
        paths.clone(),
    )
    .unwrap();
    let mut state = State::new_for_tests(FrontendContext {
        gpu_factory: Rc::new(NoRenderer),
        registry: registry.clone(),
        rom_loader: Box::new(NoRomLoader),
        audio_registry: audio.clone(),
    })
    .unwrap();
    *state.host.session_mut() = session;
    let mut baseline = state.settings_snapshot().clone();
    baseline.shared.general.language = AppLanguage::English;
    baseline.local.audio.muted = false;
    state.apply_settings(baseline.clone()).unwrap();
    let state = Rc::new(RefCell::new(state));
    let app = gtk::Application::builder()
        .application_id("org.nerust.preferences-test")
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();
    let parent = gtk::ApplicationWindow::new(&app);
    parent.present();
    pump();
    let closes = Rc::new(Cell::new(0));
    let dialog = present_preferences_dialog(&parent, state.clone(), {
        let closes = closes.clone();
        let weak = Rc::downgrade(&state);
        move || {
            assert!(weak.upgrade().unwrap().try_borrow_mut().is_ok());
            closes.set(closes.get() + 1);
        }
    })
    .expect("production preferences dialog");
    let widgets = descendants(dialog.upcast_ref());
    let mute = widgets
        .iter()
        .find_map(|widget| {
            widget
                .clone()
                .downcast::<gtk::CheckButton>()
                .ok()
                .filter(|check| {
                    check.label().as_deref()
                        == Some(nerust_settings_core::i18n::text(
                            AppLanguage::English,
                            nerust_settings_core::i18n::UiText::Mute,
                        ))
                })
        })
        .expect("production mute control");
    let volume = widgets
        .iter()
        .find_map(|widget| {
            widget
                .clone()
                .downcast::<gtk::SpinButton>()
                .ok()
                .filter(|spin| spin.range() == (0.0, 100.0))
        })
        .expect("production volume control");

    mute.set_active(true);
    volume.set_value(23.0);
    // A directory at the destination makes atomic replacement fail even as
    // root. Unlike permission changes, this reliably exercises apply_settings.
    std::fs::remove_file(&paths.settings_file).unwrap();
    std::fs::create_dir(&paths.settings_file).unwrap();
    dialog.response(gtk::ResponseType::Ok);
    pump();
    assert!(
        dialog.is_visible(),
        "save failure must leave the dialog open"
    );
    assert_eq!(closes.get(), 0);
    assert_eq!(state.borrow().settings_snapshot(), &baseline);
    assert!(mute.is_active(), "failed apply must preserve the draft");
    assert_eq!(volume.value(), 23.0);
    let error = descendants(dialog.content_area().upcast_ref())
        .into_iter()
        .find_map(|widget| {
            widget
                .downcast::<gtk::Label>()
                .ok()
                .filter(|label| label.text().starts_with("settings:"))
        })
        .expect("visible production apply_settings error");
    assert!(error.is_visible());
    assert!(error.text().starts_with("settings:"), "{}", error.text());

    volume.set_value(37.0);
    pump();
    assert!(error.text().is_empty(), "editing clears the apply error");
    assert!(!error.is_visible());
    assert_eq!(closes.get(), 0);
    std::fs::remove_dir(&paths.settings_file).unwrap();
    dialog.response(gtk::ResponseType::Ok);
    pump();
    let mut expected = baseline;
    expected.local.audio.muted = true;
    expected.local.audio.master_volume_percent = 37;
    assert_eq!(state.borrow().settings_snapshot(), &expected);
    let reloaded =
        SessionHandle::new_with_settings_paths(capabilities, registry, audio, paths).unwrap();
    assert_eq!(reloaded.settings_snapshot(), &expected);
    assert!(!dialog.is_visible());
    assert_eq!(closes.get(), 1);
    // The successful response already destroyed/unrealized the dialog.
    // Repeated destruction, then parent teardown, must not finish it twice.
    dialog.destroy();
    parent.destroy();
    pump();
    assert_eq!(state.borrow().settings_snapshot(), &expected);
    assert_eq!(closes.get(), 1);
}

#[test]
fn preferences_finish_runs_once_even_when_close_reenters() {
    let count = Rc::new(Cell::new(0));
    let callback: FinishCallback = Rc::new(RefCell::new(None));
    *callback.borrow_mut() = Some(Box::new({
        let weak = Rc::downgrade(&callback);
        let count = count.clone();
        move || {
            count.set(count.get() + 1);
            finish_preferences(&weak.upgrade().unwrap());
        }
    }));
    finish_preferences(&callback);
    finish_preferences(&callback);
    assert_eq!(count.get(), 1);
}

#[test]
fn preferences_native_close_and_destroy_finish_once() {
    assert!(
        crate::test_support::ui_test_available(),
        "real GTK display required"
    );
    for destroy in [false, true] {
        let dialog = gtk::Dialog::new();
        let count = Rc::new(Cell::new(0));
        let callback: FinishCallback = Rc::new(RefCell::new(Some(Box::new({
            let count = count.clone();
            let dialog = dialog.downgrade();
            move || {
                count.set(count.get() + 1);
                if let Some(dialog) = dialog.upgrade() {
                    dialog.destroy();
                }
            }
        }))));
        connect_preferences_finish(&dialog, &callback);
        dialog.present();
        if destroy {
            dialog.destroy();
        } else {
            dialog.close();
        }
        assert_eq!(count.get(), 1);
        dialog.destroy();
        finish_preferences(&callback);
        assert_eq!(count.get(), 1);
    }
}

#[test]
fn refreshing_guard_restores_previous_nested_state() {
    let flag = Cell::new(false);
    {
        let _outer = FlagGuard::enter(&flag);
        {
            let _inner = FlagGuard::enter(&flag);
            assert!(flag.get());
        }
        assert!(flag.get());
    }
    assert!(!flag.get());
}
