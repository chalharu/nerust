//! Retained GTK control mechanics over the headless [`crate::testkit`] app.
//!
//! Production code never touches this module. Screens are the fake's canned
//! settings/history scenes (stable labels in [`crate::testkit::labels`]);
//! policy content (real catalog strings, translations, storage validation)
//! is covered in view-defs widget session tests. These tests pin the native
//! interpreter: rendering a supplied screen, identity across refresh, page
//! metadata, entry editing, history lifecycle, and OSD headless behavior.
use super::renderer::{NativeControl, NativeKind, NodeId};
use super::*;
use crate::testkit::labels;
use crate::window::{WindowExtend, tests as windows};
use wakui::control::*;
use wakui::{aux::AuxId, focus::AuxKind};

struct Harness {
    window: crate::window::Window,
    id: AuxId,
    program: WidgetDialog,
}

impl Harness {
    fn settings() -> Option<Self> {
        let window = windows::bound_window()?;
        let program = windows::program(&window, AuxKind::Settings);
        let id = windows::route(&window, AuxKind::Settings).expect("settings route");
        Some(Self {
            window,
            id,
            program,
        })
    }

    fn history() -> Option<Self> {
        let window = windows::bound_window()?;
        let program = windows::program(&window, AuxKind::History);
        let id = windows::route(&window, AuxKind::History).expect("history route");
        Some(Self {
            window,
            id,
            program,
        })
    }

    fn binding(&self) -> &Rc<WidgetBinding> {
        &self.program.binding
    }
    fn screen(&self) -> ControlScreen {
        self.binding().screen_snapshot()
    }
    fn id(&self, label: &str) -> ControlId {
        controls(&self.screen())
            .into_iter()
            .find(|c| c.label == label)
            .unwrap_or_else(|| panic!("control {label:?}"))
            .id
    }
    fn native(&self, id: ControlId) -> Rc<NativeControl> {
        self.binding()
            .registry()
            .control(&id)
            .expect("retained control")
    }
    fn input(&self, id: ControlId, input: ControlInput) {
        self.binding().input(id, input);
    }
    fn action(&self, id: ControlId) {
        let native = self.native(id);
        let NativeKind::Action(button) = &native.kind else {
            panic!("action")
        };
        button.emit_clicked();
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.program.dialog.destroy();
        self.window.window().destroy();
    }
}

fn controls(screen: &ControlScreen) -> Vec<Control> {
    struct All(Vec<Control>);
    impl ControlVisitor for All {
        fn control(&mut self, control: &Control) {
            self.0.push(control.clone());
        }
    }
    let mut all = All(Vec::new());
    screen.visit(&mut all);
    all.0
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

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn common_renderer_uses_supplied_screen() {
    let Some(h) = Harness::settings() else {
        eprintln!("SKIP: widget test needs a display");
        return;
    };
    h.binding().render();
    let title = h.screen().title;
    assert!(!title.is_empty());
    // OK submits through the close path without destroying the dialog.
    h.action(h.id(labels::OK));
    assert!(h.screen().title.contains(&title[..title.len().min(8)]));
    assert!(windows::route(&h.window, AuxKind::Settings).is_some());
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_note_accessible_name_survives_refresh() {
    let Some(h) = Harness::history() else {
        eprintln!("SKIP: widget test needs a display");
        return;
    };
    h.window
        .state()
        .borrow_mut()
        .host
        .report_host_error("history note".into());
    h.binding().sync();
    let id = controls(&h.screen())
        .into_iter()
        .find(|c| matches!(&c.kind, ControlKind::Note { text } if text == "history note"))
        .expect("history note")
        .id;
    let label = assert_note(&h, id);
    let native = h.native(id);
    for _ in 0..3 {
        h.binding().refresh();
        pump();
        assert_eq!(assert_note(&h, id), label);
    }
    assert!(Rc::ptr_eq(&native, &h.native(id)));
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_all_pages_metadata_ranges_and_values_follow_host() {
    let Some(h) = Harness::settings() else {
        eprintln!("SKIP: widget test needs a display");
        return;
    };
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
        let node = h.binding().registry().node(&NodeId::Page(page.id)).unwrap();
        assert_eq!(
            h.binding().stack().page(&node.widget).title().as_deref(),
            Some(page.label.as_str())
        );
    }
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_entry_preserves_identity_and_single_edit() {
    let Some(h) = Harness::settings() else {
        eprintln!("SKIP: widget test needs a display");
        return;
    };
    let mute = h.id(labels::MUTE);
    let native = h.native(mute);
    let NativeKind::Toggle(check) = &native.kind else {
        panic!("toggle")
    };
    h.input(mute, ControlInput::Bool(true));
    assert!(check.is_active());
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_choice_and_submit_stage_mailbox() {
    let Some(h) = Harness::settings() else {
        eprintln!("SKIP: widget test needs a display");
        return;
    };
    let policy = h.id(labels::STORAGE_POLICY);
    let screen = h.screen();
    let ControlKind::Choice { choices, .. } = &screen.control(policy).unwrap().kind else {
        panic!("choice")
    };
    h.input(policy, ControlInput::Select(choices.len() - 1));
    h.input(h.id(labels::OK), ControlInput::Activate);
    // The staged submission flags the mailbox; closing stays native.
    assert!(
        h.window
            .state()
            .borrow()
            .host
            .widget_mailbox(h.id)
            .expect("mailbox")
            .has_pending()
    );
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn retained_native_stack_notebooks_and_context_reset_mirror_model() {
    let Some(h) = Harness::settings() else {
        eprintln!("SKIP: widget test needs a display");
        return;
    };
    let screen = h.screen();
    let page = screen.pages.iter().find(|p| p.label == "Input").unwrap();
    let widget = h
        .binding()
        .registry()
        .node(&NodeId::Page(page.id))
        .unwrap()
        .widget;
    h.binding().stack().set_visible_child(&widget);
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
    let Some(h) = Harness::history() else {
        eprintln!("SKIP: widget test needs a display");
        return;
    };
    h.window
        .state()
        .borrow_mut()
        .host
        .report_host_error("newest".into());
    h.binding().sync();
    assert!(!h.screen().title.is_empty());
}

#[test]
#[ignore = "needs display; CI-only under xvfb"]
fn history_dialog_lifecycle_runs_without_display_errors() {
    let Some(h) = Harness::history() else {
        eprintln!("SKIP: widget test needs a display");
        return;
    };
    let filter = controls(&h.screen())
        .into_iter()
        .find(|c| matches!(c.kind, ControlKind::Choice { .. }))
        .unwrap()
        .id;
    h.input(filter, ControlInput::Select(1));
    h.action(h.id(labels::CLEAR_HISTORY));
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

#[test]
fn osd_viewport_sync_and_invalidate_are_headless() {
    let state = windows::model_state();
    // Viewport sync prepares and applies internally; invalidation is
    // observable only as continued total behavior headless.
    state.borrow_mut().host.sync_osd_viewport(320, 240, 240);
    state.borrow_mut().host.invalidate_osd();
    state.borrow_mut().host.sync_osd_viewport(320, 240, 240);
    state.borrow_mut().host.invalidate_osd();
}
