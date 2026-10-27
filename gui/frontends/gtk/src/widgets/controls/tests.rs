//! Migrated behavioral tests exercise the production generic program, not a
//! second controller or settings-specific native binding.
use super::renderer::{NativeControl, NativeKind, NodeId};
use super::*;
use crate::window::{WindowExtend, tests as windows};
use nerust_frontend_host::{control::*, focus::AuxKind};
use nerust_settings_core::{
    i18n::{UiText, text},
    model::language::AppLanguage,
};

struct Harness {
    window: crate::window::Window,
    program: WidgetDialog,
}

impl Harness {
    fn new(system: bool) -> Self {
        let window = windows::bound_window().expect("isolated Xvfb process required");
        let state = window.state();
        if system {
            state.borrow_mut().ctx.registry =
                std::sync::Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![
                    nerust_nes_factory::NesFactory::boxed().into(),
                ]));
        }
        let mut baseline = state.borrow().settings_snapshot().clone();
        baseline.shared.general.language = AppLanguage::English;
        baseline.local.audio.muted = false;
        state.borrow_mut().host.apply_settings(baseline).unwrap();
        let program = windows::program(&window, AuxKind::Settings);
        Self { window, program }
    }

    fn history() -> Self {
        let window = windows::bound_window().expect("isolated Xvfb process required");
        let program = windows::program(&window, AuxKind::History);
        Self { window, program }
    }

    fn binding(&self) -> &Rc<WidgetBinding> {
        &self.program.binding
    }
    fn screen(&self) -> ControlScreen {
        self.binding().session.borrow().screen()
    }
    fn id(&self, key: UiText) -> ControlId {
        let label = text(AppLanguage::English, key);
        controls(&self.screen())
            .into_iter()
            .find(|c| c.label == label)
            .expect(label)
            .id
    }
    fn native(&self, id: ControlId) -> Rc<NativeControl> {
        self.binding()
            .registry
            .borrow()
            .control(&id)
            .expect("retained control")
    }
    fn input(&self, id: ControlId, input: ControlInput) {
        self.binding().input(id, input);
    }
    fn custom_directory(&self) -> ControlId {
        let policy = self.id(UiText::SaveStoragePolicy);
        let screen = self.screen();
        let ControlKind::Choice { choices, .. } = &screen.control(policy).unwrap().kind else {
            panic!("choice")
        };
        self.input(policy, ControlInput::Select(choices.len() - 1));
        self.id(UiText::SaveStorageDirectory)
    }
    fn action(&self, id: ControlId) {
        let native = self.native(id);
        let NativeKind::Action(button) = &native.kind else {
            panic!("action")
        };
        button.emit_clicked();
    }
    fn japanese(&self, id: ControlId) {
        let screen = self.screen();
        let ControlKind::Choice { choices, .. } = &screen.control(id).unwrap().kind else {
            panic!("choice")
        };
        let index = choices
            .iter()
            .position(|label| label.contains("日本"))
            .unwrap();
        self.input(id, ControlInput::Select(index));
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.program.dialog.destroy();
        self.window.window().destroy();
    }
}

fn controls(screen: &ControlScreen) -> Vec<Control> {
    struct Collect(Vec<Control>);
    impl ControlVisitor for Collect {
        fn control(&mut self, control: &Control) {
            self.0.push(control.clone());
        }
    }
    let mut collect = Collect(Vec::new());
    screen.visit(&mut collect);
    collect.0
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
    unsafe extern "C" {
        fn gtk_test_accessible_check_property(
            accessible: *mut std::ffi::c_void,
            property: gtk::ffi::GtkAccessibleProperty,
            ...
        ) -> *mut std::ffi::c_char;
    }
    let expected = expected.to_value();
    // SAFETY: GTK's variadic label property expects a live C string; the
    // returned mismatch is transfer-full and must be freed even on failure.
    let mismatch = unsafe {
        let actual = gtk_test_accessible_check_property(
            widget.as_ptr().cast(),
            gtk::AccessibleProperty::Label.into_glib(),
            glib::gobject_ffi::g_value_get_string(expected.to_glib_none().0),
        );
        if actual.is_null() {
            None
        } else {
            let result = std::ffi::CStr::from_ptr(actual)
                .to_string_lossy()
                .into_owned();
            glib::ffi::g_free(actual.cast());
            Some(result)
        }
    };
    assert!(
        mismatch.is_none(),
        "accessible label mismatch: {mismatch:?}"
    );
}

fn assert_note(h: &Harness, id: ControlId) -> String {
    let screen = h.screen();
    let c = screen.control(id).unwrap();
    let ControlKind::Note { text } = &c.kind else {
        panic!("note")
    };
    assert!(!text.is_empty());
    let native = h.native(id);
    let NativeKind::Note(label) = &native.kind else {
        panic!("note")
    };
    assert_eq!(label.text().as_str(), text);
    assert_accessible_label(&native.input, &c.metadata.accessible_label);
    assert_accessible_label(&native.root, &c.metadata.accessible_label);
    c.metadata.accessible_label.clone()
}

fn note_refreshes(h: &Harness, id: ControlId, translated: bool) {
    let native = h.native(id);
    let mute = h.id(UiText::Mute);
    let language = h.id(UiText::Language);
    let english = assert_note(h, id);
    h.input(mute, ControlInput::Bool(true));
    for _ in 0..3 {
        h.binding().refresh();
        pump();
        assert_eq!(assert_note(h, id), english);
    }
    h.japanese(language);
    let japanese = assert_note(h, id);
    if translated {
        assert_ne!(japanese, english);
    }
    h.input(mute, ControlInput::Bool(false));
    for _ in 0..3 {
        h.binding().refresh();
        pump();
        assert_eq!(assert_note(h, id), japanese);
    }
    assert!(Rc::ptr_eq(&native, &h.native(id)));
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn common_renderer_uses_supplied_draft_not_state_or_window_id() {
    let h = Harness::new(false);
    let mute = h.id(UiText::Mute);
    let volume = h.id(UiText::MasterVolume);
    let submit = h.id(UiText::Ok);
    h.input(mute, ControlInput::Bool(true));
    h.input(volume, ControlInput::Number(19));
    let mut owned = h.binding().session.borrow().window();
    owned.id = nerust_frontend_host::window::WindowId::Main;
    owned.title = "supplied outer chrome".into();
    {
        let _signals = FlagGuard::enter(&h.binding().refreshing);
        owned.visit(&mut crate::window::GtkWindowRenderer::Controls(h.binding()));
    }
    assert_eq!(
        h.program.dialog.title().as_deref(),
        Some("supplied outer chrome")
    );
    h.action(submit);
    let state = h.window.state();
    assert!(state.borrow().settings_snapshot().local.audio.muted);
    assert_eq!(
        state
            .borrow()
            .settings_snapshot()
            .local
            .audio
            .master_volume_percent,
        19
    );
    assert!(crate::surface_state::surface_gone(&h.program.dialog));
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn owned_outer_window_interprets_chrome_without_session_borrow() {
    let h = Harness::new(false);
    let mut owned = h.binding().session.borrow().window();
    owned.title = "outer chrome".into();
    let nerust_frontend_host::window::WidgetContents::Controls(screen) =
        &mut owned.contents.contents
    else {
        panic!("controls")
    };
    screen.title = "nested screen must not overwrite chrome".into();
    let called = Rc::new(Cell::new(0));
    let signal = h.program.dialog.connect_title_notify({
        let weak = Rc::downgrade(h.binding());
        let called = called.clone();
        move |_| {
            let b = weak.upgrade().unwrap();
            assert!(b.session.try_borrow_mut().is_ok());
            assert!(b.registry.try_borrow_mut().is_ok());
            called.set(called.get() + 1);
        }
    });
    owned.visit(&mut crate::window::GtkWindowRenderer::Controls(h.binding()));
    assert_eq!(called.get(), 1);
    assert_eq!(h.program.dialog.title().as_deref(), Some("outer chrome"));
    h.program.dialog.disconnect(signal);
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn outer_window_title_reentry_refreshes_latest_session_tree() {
    let h = Harness::new(false);
    let language = h.id(UiText::Language);
    let mute = h.id(UiText::Mute);
    let called = Rc::new(Cell::new(false));
    h.program.dialog.connect_title_notify({
        let weak = Rc::downgrade(h.binding());
        let called = called.clone();
        move |_| {
            let b = weak.upgrade().unwrap();
            assert!(b.session.try_borrow_mut().is_ok());
            assert!(b.registry.try_borrow_mut().is_ok());
            if !called.replace(true) {
                b.send(ControlEvent::new(mute, ControlInput::Bool(true)));
            }
        }
    });
    h.japanese(language);
    assert!(called.get());
    assert_eq!(
        h.program.dialog.title().as_deref(),
        Some(h.screen().title.as_str())
    );
    assert!(matches!(
        h.screen().control(mute).unwrap().kind,
        ControlKind::Toggle { value: true }
    ));
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_no_systems_accessible_name_survives_refresh_and_translation() {
    let h = Harness::new(false);
    let id = controls(&h.screen())
        .into_iter()
        .find(|c| matches!(&c.kind, ControlKind::Note { text } if text == "No systems available"))
        .unwrap()
        .id;
    note_refreshes(&h, id, true);
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_storage_error_accessible_name_survives_refresh_and_translation() {
    let h = Harness::new(false);
    let directory = h.custom_directory();
    h.input(directory, ControlInput::Text(String::new()));
    let id = controls(&h.screen()).into_iter().find(|c| c.metadata.visible && matches!(&c.kind, ControlKind::Note { text } if !text.is_empty() && c.label.is_empty() && text != "No systems available")).unwrap().id;
    note_refreshes(&h, id, false);
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_labelled_note_accessible_name_includes_value_after_translation() {
    let h = Harness::new(false);
    let position = h.id(UiText::NotificationPosition);
    let status = h.id(UiText::NotificationStatusPosition);
    h.input(position, ControlInput::Select(0));
    h.input(status, ControlInput::Select(0));
    let c = controls(&h.screen())
        .into_iter()
        .find(|c| {
            c.metadata.visible && !c.label.is_empty() && matches!(c.kind, ControlKind::Note { .. })
        })
        .unwrap();
    let ControlKind::Note { text } = &c.kind else {
        unreachable!()
    };
    assert_eq!(c.metadata.accessible_label, format!("{}: {text}", c.label));
    note_refreshes(&h, c.id, true);
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_all_pages_metadata_ranges_and_values_follow_host() {
    let h = Harness::new(true);
    let screen = h.screen();
    assert_eq!(screen.pages.len(), 6);
    for c in controls(&screen) {
        let native = h.native(c.id);
        assert_eq!(native.root.property::<bool>("visible"), c.metadata.visible);
        assert_eq!(native.input.is_sensitive(), c.metadata.enabled);
        assert_accessible_label(&native.input, &c.metadata.accessible_label);
        match (&native.kind, &c.kind) {
            (NativeKind::Toggle(check), ControlKind::Toggle { value }) => {
                assert_eq!(check.is_active(), *value)
            }
            (NativeKind::Choice(combo, _), ControlKind::Choice { selected, .. }) => {
                assert_eq!(combo.active(), selected.map(|i| i as u32))
            }
            (
                NativeKind::Number(spin, label),
                ControlKind::Number {
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
            .binding()
            .registry
            .borrow()
            .node(&NodeId::Page(page.id))
            .unwrap();
        assert_eq!(
            h.binding().stack.page(&node.widget).title().as_deref(),
            Some(page.label.as_str())
        );
    }
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_entry_preserves_identity_cursor_selection_focus_and_single_edit() {
    let h = Harness::new(false);
    let directory = h.custom_directory();
    let mute = h.id(UiText::Mute);
    let native = h.native(directory);
    let NativeKind::Text(entry, _) = &native.kind else {
        panic!("text")
    };
    entry.set_text("/tmp/unfinished-directory");
    pump();
    assert!(entry.grab_focus());
    entry.select_region(2, 7);
    let selection = entry.selection_bounds();
    let position = entry.position();
    let focus = GtkWindowExt::focus(&h.program.dialog).unwrap();
    let edits = Rc::new(Cell::new(0));
    entry.connect_changed({
        let edits = edits.clone();
        move |_| edits.set(edits.get() + 1)
    });
    h.input(mute, ControlInput::Bool(true));
    h.binding().refresh();
    pump();
    assert!(Rc::ptr_eq(&native, &h.native(directory)));
    assert_eq!(entry.selection_bounds(), selection);
    assert_eq!(entry.position(), position);
    assert_eq!(
        GtkWindowExt::focus(&h.program.dialog).as_ref(),
        Some(&focus)
    );
    assert_eq!(edits.get(), 0, "unrelated refresh must not reset text/IME");
    entry.select_region(0, 0);
    entry.insert_text("x", &mut 3);
    assert_eq!(edits.get(), 1);
    assert!(
        matches!(&h.screen().control(directory).unwrap().kind, ControlKind::Text { value, .. } if value == entry.text().as_str())
    );
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_same_widgets_update_values_translation_and_validation() {
    let h = Harness::new(false);
    let mute = h.id(UiText::Mute);
    let language = h.id(UiText::Language);
    let submit = h.id(UiText::Ok);
    let policy = h.id(UiText::SaveStoragePolicy);
    let directory = h.id(UiText::SaveStorageDirectory);
    let native = h.native(mute);
    h.input(mute, ControlInput::Bool(true));
    h.japanese(language);
    assert!(Rc::ptr_eq(&native, &h.native(mute)));
    let NativeKind::Toggle(check) = &native.kind else {
        panic!("toggle")
    };
    assert_eq!(
        check.label().as_deref(),
        Some(h.screen().control(mute).unwrap().label.as_str())
    );
    let screen = h.screen();
    let ControlKind::Choice { choices, .. } = &screen.control(policy).unwrap().kind else {
        panic!("choice")
    };
    h.input(policy, ControlInput::Select(choices.len() - 1));
    h.input(directory, ControlInput::Text(String::new()));
    assert!(!h.native(submit).input.is_sensitive());
    assert!(!h.binding().error_label.text().is_empty());
    h.input(submit, ControlInput::Activate);
    assert!(!h.binding().session.borrow().mailbox().has_pending());
    h.input(
        directory,
        ControlInput::Text(std::env::temp_dir().to_string_lossy().into()),
    );
    assert!(h.native(submit).input.is_sensitive());
    assert!(h.binding().error_label.text().is_empty());
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_native_stack_notebooks_and_context_reset_mirror_model() {
    let h = Harness::new(true);
    let screen = h.screen();
    let page = screen
        .pages
        .iter()
        .find(|p| p.label == text(AppLanguage::English, UiText::Input))
        .unwrap();
    let widget = h
        .binding()
        .registry
        .borrow()
        .node(&NodeId::Page(page.id))
        .unwrap()
        .widget;
    h.binding().stack.set_visible_child(&widget);
    assert!(
        h.screen()
            .pages
            .iter()
            .find(|p| p.id == page.id)
            .unwrap()
            .selected
    );
    let notebooks: Vec<gtk::Notebook> = h
        .binding()
        .registry
        .borrow()
        .nodes
        .iter()
        .filter(|(id, _)| matches!(id, NodeId::Tabs(_)))
        .map(|(_, n)| n.widget.clone().downcast().unwrap())
        .collect();
    let inputs = notebooks.iter().find(|n| n.n_pages() >= 2).unwrap();
    inputs.set_current_page(Some(inputs.n_pages() - 1));
    pump();
    assert_eq!(inputs.current_page(), Some(inputs.n_pages() - 1));
    inputs.set_current_page(Some(0));
    pump();
    assert_eq!(inputs.current_page(), Some(0));
    let audio = screen
        .pages
        .iter()
        .find(|p| p.label == text(AppLanguage::English, UiText::Audio))
        .unwrap();
    h.input(audio.select.id, ControlInput::Activate);
    let widget = h
        .binding()
        .registry
        .borrow()
        .node(&NodeId::Page(audio.id))
        .unwrap()
        .widget;
    assert_eq!(h.binding().stack.visible_child(), Some(widget));
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_binding_capture_key_clear_and_dynamic_topology() {
    let h = Harness::new(true);
    let c = controls(&h.screen())
        .into_iter()
        .rev()
        .find(|c| matches!(c.kind, ControlKind::Binding { .. }))
        .unwrap();
    let native = h.native(c.id);
    let NativeKind::Binding(value, change, clear) = &native.kind else {
        panic!("binding")
    };
    change.emit_clicked();
    assert_eq!(h.screen().capture.target, Some(c.id));
    let controllers = h.program.dialog.observe_controllers();
    let keys = (0..controllers.n_items())
        .find_map(|i| {
            controllers
                .item(i)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .unwrap();
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[&gdk::Key::F2, &0u32, &gdk::ModifierType::empty()]
    ));
    assert!(h.screen().capture.target.is_none());
    assert_eq!(value.text().as_str(), "F2");
    clear.emit_clicked();
    assert!(
        matches!(&h.screen().control(c.id).unwrap().kind, ControlKind::Binding { text, capturing: false, .. } if text != "F2")
    );
    let before = h.binding().registry.borrow().controls.len();
    let slot = controls(&h.screen()).into_iter().find(|c| matches!(&c.kind, ControlKind::Choice { choices, .. } if choices.iter().any(|label| label.contains("None") || label.contains("Disconnected"))));
    // Slot choice comes from the current input subtree, not a settings ID.
    let slot = slot.unwrap_or_else(|| {
        fn choice(l: &Layout) -> Option<Control> {
            l.children.iter().find_map(|n| match n {
                Node::Control(c) if matches!(c.kind, ControlKind::Choice { .. }) => Some(c.clone()),
                Node::Row(l) | Node::Column(l) | Node::Group(l) => choice(l),
                Node::Tabs(t) => t.tabs.iter().find_map(|t| choice(&t.body)),
                _ => None,
            })
        }
        let screen = h.screen();
        choice(
            &screen
                .pages
                .iter()
                .find(|p| p.label == text(AppLanguage::English, UiText::Input))
                .unwrap()
                .body,
        )
        .unwrap()
    });
    h.input(slot.id, ControlInput::Select(0));
    assert!(h.binding().registry.borrow().controls.len() < before);
    assert!(Rc::ptr_eq(&native, &h.native(c.id)));
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_browse_is_accessible_nonblocking_and_cancel_preserves_text() {
    let h = Harness::new(false);
    let directory = h.custom_directory();
    h.input(directory, ControlInput::Text("/tmp".into()));
    let native = h.native(directory);
    let NativeKind::Text(entry, browse) = &native.kind else {
        panic!("text")
    };
    assert!(browse.property::<bool>("visible"));
    assert_accessible_label(browse.upcast_ref(), browse.label().unwrap().as_str());
    browse.emit_clicked();
    let chooser = h
        .binding()
        .chooser
        .borrow()
        .clone()
        .expect("nonblocking chooser");
    assert_eq!(chooser.action(), gtk::FileChooserAction::SelectFolder);
    fn cancel(chooser: &gtk::FileChooserNative) {
        chooser.emit_by_name::<()>(
            "response",
            &[&glib::translate::IntoGlib::into_glib(
                gtk::ResponseType::Cancel,
            )],
        );
    }
    cancel(&chooser);
    assert!(h.binding().chooser.borrow().is_none());
    assert_eq!(entry.text().as_str(), "/tmp");
    browse.emit_clicked();
    let next = h.binding().chooser.borrow().clone().unwrap();
    cancel(&chooser);
    assert_eq!(h.binding().chooser.borrow().as_ref(), Some(&next));
    cancel(&next);
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_synchronous_setter_reentry_is_borrow_free_and_latest_wins() {
    let h = Harness::new(false);
    let mute = h.id(UiText::Mute);
    let native = h.native(mute);
    let NativeKind::Toggle(check) = &native.kind else {
        panic!("toggle")
    };
    let called = Rc::new(Cell::new(false));
    check.connect_toggled({
        let weak = Rc::downgrade(h.binding());
        let called = called.clone();
        move |_| {
            let b = weak.upgrade().unwrap();
            assert!(b.session.try_borrow_mut().is_ok());
            assert!(b.registry.try_borrow_mut().is_ok());
            if !called.replace(true) {
                b.send(ControlEvent::new(mute, ControlInput::Bool(false)));
                assert!(
                    matches!(
                        b.session.borrow().screen().control(mute).unwrap().kind,
                        ControlKind::Toggle { value: true }
                    ),
                    "reentrant input must wait until native traversal completes"
                );
                b.refresh();
            }
        }
    });
    h.input(mute, ControlInput::Bool(true));
    assert!(called.get());
    assert!(!check.is_active());
    assert!(matches!(
        h.screen().control(mute).unwrap().kind,
        ControlKind::Toggle { value: false }
    ));
    assert!(!h.binding().refreshing.get());
    assert!(!h.binding().updating.get());
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_scheduled_dispatch_defers_and_weak_idle_does_not_retain_binding() {
    let h = Harness::new(false);
    let mute = h.id(UiText::Mute);
    let native = h.native(mute);
    let NativeKind::Toggle(check) = &native.kind else {
        panic!("toggle")
    };
    {
        let _session = h.binding().session.borrow_mut();
        h.binding()
            .send(ControlEvent::new(mute, ControlInput::Bool(true)));
        assert!(h.binding().scheduled.get());
        assert!(!check.is_active());
    }
    pump();
    assert!(check.is_active());
    assert!(!h.binding().scheduled.get());
    let weak = Rc::downgrade(h.binding());
    h.binding().schedule();
    drop(h);
    assert!(weak.upgrade().is_none());
    pump();
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn settings_production_action_save_failure_edit_retry_closes_once() {
    use nerust_frontend_kernel::pause::PauseOwner;
    use nerust_gui_runtime::settings::{
        HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
    };
    use nerust_gui_shell::session::SessionHandle;
    let window = windows::bound_window().expect("Xvfb");
    let state = window.state();
    let root = std::env::temp_dir().join(format!("nerust-generic-retry-{}", std::process::id()));
    let paths = SettingsPaths {
        settings_file: root.join("settings.yaml"),
        central_storage_root: root.join("storage"),
    };
    let capabilities = HostBackendCapabilities {
        window: HostWindowCapabilities {
            remembers_window_size: false,
            supports_fullscreen_default: true,
            supports_scaling: true,
        },
        presentation: None,
    };
    let registry = state.borrow().ctx.registry.clone();
    let audio = state.borrow().ctx.audio_registry.clone();
    *state.borrow_mut().host.session_mut() = SessionHandle::new_with_settings_paths(
        capabilities,
        registry.clone(),
        audio.clone(),
        paths.clone(),
    )
    .unwrap();
    let mut baseline = state.borrow().settings_snapshot().clone();
    baseline.shared.general.language = AppLanguage::English;
    baseline.local.audio.muted = false;
    state
        .borrow_mut()
        .host
        .apply_settings(baseline.clone())
        .unwrap();
    let program = windows::program(&window, AuxKind::Settings);
    let h = Harness { window, program };
    let id = windows::route(&h.window, AuxKind::Settings).unwrap();
    let mute = h.id(UiText::Mute);
    let volume = h.id(UiText::MasterVolume);
    let submit = h.id(UiText::Ok);
    let entry = h.native(volume);
    h.input(mute, ControlInput::Bool(true));
    h.input(volume, ControlInput::Number(23));
    std::fs::remove_file(&paths.settings_file).unwrap();
    std::fs::create_dir(&paths.settings_file).unwrap();
    h.action(submit);
    pump();
    assert!(h.program.dialog.is_visible());
    assert_eq!(windows::route(&h.window, AuxKind::Settings), Some(id));
    assert!(state.borrow().host.owns_pause(PauseOwner::Settings));
    assert_eq!(state.borrow().settings_snapshot(), &baseline);
    assert!(matches!(
        h.screen().control(volume).unwrap().kind,
        ControlKind::Number { value: 23, .. }
    ));
    assert!(h.binding().error_label.text().starts_with("settings:"));
    assert!(!h.binding().session.borrow().mailbox().close_requested());
    h.input(volume, ControlInput::Number(37));
    assert!(h.binding().error_label.text().is_empty());
    assert!(Rc::ptr_eq(&entry, &h.native(volume)));
    std::fs::remove_dir(&paths.settings_file).unwrap();
    h.action(submit);
    pump();
    let mut expected = baseline;
    expected.local.audio.muted = true;
    expected.local.audio.master_volume_percent = 37;
    assert_eq!(state.borrow().settings_snapshot(), &expected);
    let reloaded =
        SessionHandle::new_with_settings_paths(capabilities, registry, audio, paths).unwrap();
    assert_eq!(reloaded.settings_snapshot(), &expected);
    assert!(!h.program.dialog.is_visible());
    assert!(windows::route(&h.window, AuxKind::Settings).is_none());
    assert!(!state.borrow().host.owns_pause(PauseOwner::Settings));
    h.program.dialog.destroy();
    h.window.window().destroy();
    assert_eq!(state.borrow().settings_snapshot(), &expected);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn native_finish_runs_once_even_when_close_reenters() {
    use crate::native::dialog::{FinishCallback, finish};
    let count = Rc::new(Cell::new(0));
    let callback: FinishCallback = Rc::new(RefCell::new(None));
    *callback.borrow_mut() = Some(Box::new({
        let weak = Rc::downgrade(&callback);
        let count = count.clone();
        move || {
            count.set(count.get() + 1);
            finish(&weak.upgrade().unwrap());
        }
    }));
    finish(&callback);
    finish(&callback);
    assert_eq!(count.get(), 1);
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn native_close_cancel_and_parent_destroy_discard_once() {
    use nerust_frontend_kernel::pause::PauseOwner;
    for mode in 0..3 {
        let h = Harness::new(false);
        let mute = h.id(UiText::Mute);
        h.input(mute, ControlInput::Bool(true));
        let state = h.window.state();
        let before = state.borrow().settings_snapshot().clone();
        match mode {
            0 => h.program.dialog.close(),
            1 => h.action(h.id(UiText::Cancel)),
            _ => h.window.window().destroy(),
        }
        pump();
        assert_eq!(state.borrow().settings_snapshot(), &before);
        assert!(
            windows::route(&h.window, AuxKind::Settings).is_none(),
            "close mode {mode} must remove the route"
        );
        assert!(!state.borrow().host.owns_pause(PauseOwner::Settings));
        h.program.dialog.destroy();
        assert_eq!(state.borrow().settings_snapshot(), &before);
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

fn history_choice(h: &Harness) -> ControlId {
    controls(&h.screen())
        .into_iter()
        .find(|c| matches!(c.kind, ControlKind::Choice { .. }))
        .unwrap()
        .id
}

fn history_table(h: &Harness) -> (LayoutId, Vec<Layout>) {
    let screen = h.screen();
    let Node::Table(table) = screen.pages[0]
        .body
        .children
        .iter()
        .find(|n| matches!(n, Node::Table(_)))
        .unwrap()
    else {
        unreachable!()
    };
    (
        table.id,
        table
            .children
            .iter()
            .filter_map(|n| {
                if let Node::Row(l) = n {
                    Some(l.clone())
                } else {
                    None
                }
            })
            .collect(),
    )
}

fn assert_history(h: &Harness) {
    let screen = h.screen();
    assert_eq!(
        h.program.dialog.title().as_deref(),
        Some(screen.title.as_str())
    );
    for c in controls(&screen) {
        let native = h.native(c.id);
        match (&native.kind, &c.kind) {
            (NativeKind::Choice(combo, _), ControlKind::Choice { selected, .. }) => {
                assert_eq!(combo.active(), selected.map(|n| n as u32))
            }
            (NativeKind::Action(button), ControlKind::Action) => {
                assert_eq!(button.label().as_deref(), Some(c.label.as_str()))
            }
            (NativeKind::Note(label), ControlKind::Note { text }) => {
                assert_eq!(label.text().as_str(), text)
            }
            _ => panic!("History must use the shared choice/action/note interpreter"),
        }
    }
    let (id, rows) = history_table(h);
    let grid = h
        .binding()
        .registry
        .borrow()
        .node(&NodeId::Layout(id, 3))
        .unwrap()
        .widget
        .downcast::<gtk::Grid>()
        .unwrap();
    for (index, row) in rows.iter().enumerate() {
        let row_widget = h
            .binding()
            .registry
            .borrow()
            .node(&NodeId::Layout(row.id, 0))
            .unwrap()
            .widget;
        assert_eq!(grid.child_at(0, index as i32), Some(row_widget));
    }
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn history_captions_follow_language() {
    let h = Harness::history();
    let english = h.screen();
    let state = h.window.state();
    let mut settings = state.borrow().settings_snapshot().clone();
    settings.shared.general.language = AppLanguage::Japanese;
    state.borrow_mut().host.apply_settings(settings).unwrap();
    h.binding().sync();
    assert_ne!(h.screen().title, english.title);
    assert_history(&h);
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn history_dialog_lifecycle_runs_without_display_errors() {
    let h = Harness::history();
    let filter = history_choice(&h);
    h.input(filter, ControlInput::Select(1));
    h.action(h.id(UiText::HistoryClear));
    h.binding().sync();
    assert_history(&h);
    let close = controls(&h.screen())
        .into_iter()
        .rev()
        .find(|c| matches!(c.kind, ControlKind::Action))
        .unwrap()
        .id;
    h.action(close);
    assert!(windows::route(&h.window, AuxKind::History).is_none());
    let rebuilt = windows::program(&h.window, AuxKind::History);
    assert_ne!(rebuilt.dialog, h.program.dialog);
    rebuilt.dialog.destroy();
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn history_dialog_shows_new_sorted_rows_and_retains_existing_rows() {
    let h = Harness::history();
    let state = h.window.state();
    state
        .borrow_mut()
        .host
        .report_host_error("older <row> & detail".into());
    h.binding().sync();
    let (_, first) = history_table(&h);
    let existing = first[1].id;
    let native = h
        .binding()
        .registry
        .borrow()
        .node(&NodeId::Layout(existing, 0))
        .unwrap()
        .widget;
    state
        .borrow_mut()
        .host
        .report_host_error("newer row".into());
    h.binding().sync();
    let (_, rows) = history_table(&h);
    assert!(rows.len() > first.len());
    assert_eq!(
        h.binding()
            .registry
            .borrow()
            .node(&NodeId::Layout(existing, 0))
            .unwrap()
            .widget,
        native
    );
    let values: Vec<String> = rows
        .iter()
        .skip(1)
        .map(|row| {
            let Node::Control(c) = row.children.last().unwrap() else {
                panic!("cell")
            };
            let ControlKind::Note { text } = &c.kind else {
                panic!("note")
            };
            text.clone()
        })
        .collect();
    let model = nerust_frontend_host::history::HistoryModel::build(
        state
            .borrow_mut()
            .host
            .session_mut()
            .notification_history_lines_cached(AppLanguage::English),
        AppLanguage::English,
        nerust_gui_shell::session::osd::HistoryFilter::All,
    );
    assert_eq!(
        values,
        model
            .rows
            .iter()
            .map(|r| r.message.clone())
            .collect::<Vec<_>>()
    );
    assert_history(&h);
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn host_history_refresh_preserves_filter_and_clear_is_actual_submission() {
    let h = Harness::history();
    let filter = history_choice(&h);
    h.input(filter, ControlInput::Select(2));
    let state = h.window.state();
    state.borrow_mut().host.report_host_error("clear me".into());
    h.binding().sync();
    assert!(history_table(&h).1.len() > 1);
    h.action(h.id(UiText::HistoryClear));
    h.binding().sync();
    assert_eq!(history_table(&h).1.len(), 1);
    assert!(matches!(
        h.screen().control(filter).unwrap().kind,
        ControlKind::Choice {
            selected: Some(2),
            ..
        }
    ));
    assert!(h.program.dialog.is_visible());
    assert!(
        state
            .borrow_mut()
            .host
            .session_mut()
            .notification_history_lines_cached(AppLanguage::English)
            .is_empty()
    );
    assert_history(&h);
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn native_history_title_reentry_is_borrow_free_and_latest_model_wins() {
    let h = Harness::history();
    let called = Rc::new(Cell::new(false));
    let state = h.window.state();
    h.program.dialog.connect_title_notify({
        let weak = Rc::downgrade(h.binding());
        let state = state.clone();
        let called = called.clone();
        move |_| {
            let b = weak.upgrade().unwrap();
            assert!(b.session.try_borrow_mut().is_ok());
            assert!(b.registry.try_borrow_mut().is_ok());
            assert!(state.try_borrow_mut().is_ok());
            if called.replace(true) {
                return;
            }
            let mut s = state.borrow().settings_snapshot().clone();
            s.shared.general.language = AppLanguage::English;
            state.borrow_mut().host.apply_settings(s).unwrap();
            state
                .borrow_mut()
                .host
                .report_host_error("newest <row> & detail".into());
            b.sync();
        }
    });
    let mut settings = state.borrow().settings_snapshot().clone();
    settings.shared.general.language = AppLanguage::Japanese;
    state.borrow_mut().host.apply_settings(settings).unwrap();
    h.binding().sync();
    assert!(called.get());
    assert_eq!(
        h.screen().title,
        text(AppLanguage::English, UiText::NotificationHistory)
    );
    assert_history(&h);
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn native_history_filter_reentry_keeps_newest_selection() {
    let h = Harness::history();
    let filter = history_choice(&h);
    let native = h.native(filter);
    let NativeKind::Choice(combo, _) = &native.kind else {
        panic!("choice")
    };
    let called = Rc::new(Cell::new(false));
    combo.connect_changed({
        let called = called.clone();
        let weak = Rc::downgrade(h.binding());
        move |combo| {
            let b = weak.upgrade().unwrap();
            assert!(b.session.try_borrow_mut().is_ok());
            assert!(b.registry.try_borrow_mut().is_ok());
            if combo.active() == Some(1) && !called.replace(true) {
                combo.set_active(Some(2));
            }
        }
    });
    h.input(filter, ControlInput::Select(1));
    assert!(called.get());
    assert_eq!(combo.active(), Some(2));
    assert!(matches!(
        h.screen().control(filter).unwrap().kind,
        ControlKind::Choice {
            selected: Some(2),
            ..
        }
    ));
    assert_history(&h);
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn history_model_refresh_is_revision_gated() {
    let h = Harness::history();
    let state = h.window.state();
    let first = state
        .borrow_mut()
        .host
        .session_mut()
        .notification_history_lines_cached(AppLanguage::English)
        .as_ptr();
    let second = state
        .borrow_mut()
        .host
        .session_mut()
        .notification_history_lines_cached(AppLanguage::English)
        .as_ptr();
    assert_eq!(first, second);
    let initial = h.screen();
    h.binding().sync();
    assert_eq!(h.screen(), initial);
    state.borrow_mut().host.report_host_error("revision".into());
    h.binding().sync();
    assert_ne!(h.screen(), initial);
    assert_history(&h);
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn synthetic_debugger_ppu_scene_uses_same_opaque_interpreter() {
    let h = Harness::history();
    let mut ids = IdRegistry::<&str>::default();
    let mut control = |key, label: &str, kind| Control {
        id: ids.id(key),
        label: label.into(),
        metadata: ControlMetadata::new(label),
        display: ControlDisplay::Standard,
        kind,
    };
    let text_control = control(
        "address",
        "Memory address",
        ControlKind::Text {
            value: "C000".into(),
            placeholder: "hex".into(),
            browse_label: None,
        },
    );
    let choice = control(
        "ppu",
        "PPU layer",
        ControlKind::Choice {
            choices: vec!["BG".into(), "Sprites".into()],
            selected: Some(1),
        },
    );
    let action = control("step", "Step instruction", ControlKind::Action);
    let note = control(
        "register",
        "PC",
        ControlKind::Note {
            text: "C010".into(),
        },
    );
    let select = control("select", "Inspector", ControlKind::Action);
    let row = Layout {
        id: ids.id("row"),
        label: String::new(),
        metadata: ControlMetadata::new("register row"),
        spacing: 4,
        indent: 0,
        header: None,
        children: vec![Node::Control(note.clone())],
    };
    let table = Layout {
        id: ids.id("table"),
        children: vec![Node::Row(row)],
        ..h.screen().footer.clone()
    };
    let body = Layout {
        id: ids.id("body"),
        children: vec![
            Node::Control(text_control.clone()),
            Node::Control(choice.clone()),
            Node::Table(table),
        ],
        ..h.screen().footer.clone()
    };
    let mut screen = ControlScreen {
        title: "Debugger / PPU".into(),
        pages: vec![Page {
            id: ids.id("page"),
            label: "Inspector".into(),
            selected: true,
            select,
            body,
        }],
        footer: Layout {
            id: ids.id("footer"),
            children: vec![Node::Control(action.clone())],
            ..h.screen().footer.clone()
        },
        message: None,
        capture: Capture {
            target: None,
            prompt: String::new(),
        },
    };
    let _guard = FlagGuard::enter(&h.binding().refreshing);
    h.binding().render(&screen);
    let retained = h.native(text_control.id);
    let NativeKind::Text(entry, _) = &retained.kind else {
        panic!("generic entry")
    };
    assert_eq!(entry.text().as_str(), "C000");
    assert!(matches!(h.native(choice.id).kind, NativeKind::Choice(..)));
    assert!(matches!(h.native(action.id).kind, NativeKind::Action(_)));
    assert!(
        h.native(action.id).root.parent().is_some(),
        "footer action is not a special Dialog response"
    );
    assert!(
        h.program
            .dialog
            .widget_for_response(gtk::ResponseType::Ok)
            .is_none()
    );
    screen.pages[0].body.children[0] = Node::Control(Control {
        label: "New address".into(),
        ..text_control.clone()
    });
    h.binding().render(&screen);
    assert!(Rc::ptr_eq(&retained, &h.native(text_control.id)));
    assert_accessible_label(&retained.input, "Memory address");
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn generic_picker_rejects_stale_request_and_old_generation() {
    let h = Harness::new(false);
    let target = h.custom_directory();
    h.input(target, ControlInput::Text("/tmp".into()));
    let mut session = h.binding().session.borrow_mut();
    let first = session.dispatch(ControlEvent::new(target, ControlInput::Activate));
    let second = session.dispatch(ControlEvent::new(target, ControlInput::Activate));
    let ControlEffect::PickDirectory { request: old, .. } = first[0];
    let ControlEffect::PickDirectory { request: new, .. } = second[0];
    session.submit(target, old, Some("/stale".into()));
    assert!(
        matches!(&session.screen().control(target).unwrap().kind, ControlKind::Text { value, .. } if value == "/tmp")
    );
    session.submit(target, new, Some("/accepted".into()));
    session.submit(target, new, Some("/duplicate".into()));
    assert!(
        matches!(&session.screen().control(target).unwrap().kind, ControlKind::Text { value, .. } if value == "/accepted")
    );
    drop(session);
    h.program.dialog.destroy();
    let replacement = windows::program(&h.window, AuxKind::Settings);
    h.binding().send(ControlEvent::new(
        target,
        ControlInput::Text("/old-generation".into()),
    ));
    assert!(
        replacement
            .binding
            .session
            .borrow()
            .screen()
            .control(target)
            .is_none()
    );
    replacement.dialog.destroy();
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn generic_dispatch_uses_latest_choices_and_rejects_removed_or_disabled_ids() {
    let h = Harness::new(false);
    let directory = h.id(UiText::SaveStorageDirectory);
    let before = h.screen();
    assert!(!before.control(directory).unwrap().metadata.enabled);
    h.input(directory, ControlInput::Text("/disabled".into()));
    assert_eq!(h.screen().control(directory), before.control(directory));
    let language = h.id(UiText::Language);
    h.input(language, ControlInput::Select(usize::MAX));
    assert_eq!(h.screen(), before);
    let mut ids = IdRegistry::default();
    h.input(ids.id("not in this scene"), ControlInput::Activate);
    assert_eq!(h.screen(), before);
}

#[path = "osd_tests.rs"]
mod osd_tests;
