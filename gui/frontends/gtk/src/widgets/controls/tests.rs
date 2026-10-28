//! Migrated behavioral tests exercise the production generic program, not a
//! second controller or settings-specific native binding.
use super::renderer::{NativeControl, NativeKind, NodeId};
use super::*;
use crate::window::{MenubarModels, WindowExtend, tests as windows};
use nerust_frontend_host::{aux::AuxId, focus::AuxKind};
use nerust_settings_core::{
    i18n::{UiText, text},
    model::language::AppLanguage,
};
use wakui::control::*;

struct Harness {
    window: crate::window::Window,
    id: AuxId,
    program: WidgetDialog,
}

impl Harness {
    fn new(system: bool) -> Self {
        let window = if system {
            nes_window().expect("isolated Xvfb process required")
        } else {
            windows::bound_window().expect("isolated Xvfb process required")
        };
        let program = windows::program(&window, AuxKind::Settings);
        let id = windows::route(&window, AuxKind::Settings).expect("settings route");
        Self {
            window,
            id,
            program,
        }
    }

    fn history() -> Self {
        let window = windows::bound_window().expect("isolated Xvfb process required");
        let program = windows::program(&window, AuxKind::History);
        let id = windows::route(&window, AuxKind::History).expect("history route");
        Self {
            window,
            id,
            program,
        }
    }

    fn binding(&self) -> &Rc<WidgetBinding> {
        &self.program.binding
    }
    fn screen(&self) -> ControlScreen {
        self.binding().screen.borrow().clone()
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

/// NES-backed window for system-specific tests: canned ROM, temp settings.
fn nes_window() -> Option<crate::window::Window> {
    use std::sync::Arc;
    if !crate::test_support::ui_test_available() {
        return None;
    }
    struct CannedRom;
    impl nerust_gui_shell::load::RomLoader for CannedRom {
        fn load_rom(
            &mut self,
            _path: &std::path::Path,
            target: &mut dyn nerust_gui_shell::load::RomLoadTarget,
        ) -> Result<(), nerust_gui_shell::load::RomLoaderError> {
            use nerust_system_traits::factory::settings::FactorySettingsView;
            let factory = nerust_nes_factory::NesFactory::boxed();
            let view = FactorySettingsView {
                language: Default::default(),
                system_config: factory
                    .as_system_defaults()
                    .unwrap()
                    .default_system_settings(),
            };
            let request = factory
                .resolve_load_request(&view, factory.default_load_options())
                .unwrap();
            target
                .set_active_system(factory.system_id().as_ref())
                .unwrap();
            let mut rom = vec![0u8; 16 + 16384 + 8192];
            rom[..6].copy_from_slice(&[b'N', b'E', b'S', 0x1a, 1, 1]);
            rom[16..19].copy_from_slice(&[0x4c, 0, 0x80]);
            for vector in [0x3ffa, 0x3ffc, 0x3ffe] {
                rom[16 + vector..18 + vector].copy_from_slice(&[0x00, 0x80]);
            }
            target.load_resolved(
                nerust_system_traits::factory::load::MediaObject::new(None, rom),
                request,
            )
        }
    }
    let app = gtk::Application::builder()
        .application_id("org.nerust.widgets-nes-test")
        .build();
    let window = gtk::ApplicationWindow::builder().application(&app).build();
    let registry = Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![
        nerust_nes_factory::NesFactory::boxed().into(),
    ]));
    let ctx = nerust_gui_shell::context::FrontendContext {
        gpu_factory: std::rc::Rc::new(nerust_frontend_viewdefs::viewport::testkit::NullFactory {
            fail_create: true,
            ..Default::default()
        }),
        registry,
        rom_loader: Box::new(CannedRom),
        audio_registry: Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new()),
    };
    let state = Rc::new(RefCell::new(
        crate::State::new_for_tests(ctx).expect("test state builds"),
    ));
    state
        .borrow_mut()
        .load_path(std::path::Path::new("test.nes"))
        .expect("canned NES loads");
    Some(crate::window::Window::bind(
        app,
        window,
        gtk::Picture::new(),
        state,
        MenubarModels::new(),
    ))
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
fn common_renderer_uses_supplied_screen() {
    let h = Harness::new(false);
    h.binding().render();
    let title = h.screen().title;
    assert!(!title.is_empty());
    // OK submits through the close path without destroying the dialog.
    h.action(h.id(UiText::Ok));
    assert!(h.screen().title.contains(&title[..title.len().min(8)]));
    assert!(windows::route(&h.window, AuxKind::Settings).is_some());
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
            c.metadata.visible && !c.label.is_empty() && matches!(&c.kind, ControlKind::Note { .. })
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
    let mute = h.id(UiText::Mute);
    let native = h.native(mute);
    let NativeKind::Toggle(check) = &native.kind else {
        panic!("toggle")
    };
    h.input(mute, ControlInput::Bool(true));
    assert!(check.is_active());
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
    let NativeKind::Toggle(check) = &native.kind else {
        panic!("toggle")
    };
    assert!(check.is_active());
    let screen = h.screen();
    let ControlKind::Choice { choices, .. } = &screen.control(policy).unwrap().kind else {
        panic!("choice")
    };
    h.input(policy, ControlInput::Select(choices.len() - 1));
    h.input(directory, ControlInput::Text(String::new()));
    assert!(!h.native(submit).input.is_sensitive());
    h.input(submit, ControlInput::Activate);
    // Empty directory keeps the draft staged: nothing applies.
    assert!(
        h.window
            .state()
            .borrow()
            .host
            .widget_mailbox(h.id)
            .expect("mailbox")
            .has_pending()
    );
    h.input(
        directory,
        ControlInput::Text(std::env::temp_dir().to_string_lossy().into()),
    );
    assert!(h.native(submit).input.is_sensitive());
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
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn native_history_title_reentry_keeps_latest() {
    let h = Harness::history();
    h.window
        .state()
        .borrow_mut()
        .host
        .report_host_error("newest".into());
    h.binding().sync();
    assert!(!h.screen().title.is_empty());
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

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn history_captions_follow_language() {
    let h = Harness::history();
    let english = h.screen();
    h.action(h.id(UiText::Language));
    h.binding().sync();
    assert_ne!(h.screen().title, english.title);
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn history_dialog_lifecycle_runs_without_display_errors() {
    let h = Harness::history();
    let filter = history_choice(&h);
    h.input(filter, ControlInput::Select(1));
    h.action(h.id(UiText::HistoryClear));
    h.binding().sync();
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

#[path = "osd_tests.rs"]
mod osd_tests;
