//! Exact/ignored execution in a fresh GTK process, never alongside GTK3/Tao.
use crate::{backend, model::AppCommands};
use panechord_gtk::gtk::{self, prelude::*};
use panechord_runtime::{Policy, Store};
use std::time::Duration;

fn pump_until(condition: impl Fn() -> bool) {
    let context = gtk::glib::MainContext::default();
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "native wait timed out"
        );
        context.iteration(false);
    }
}

fn find(widget: &gtk::Widget, id: &str) -> Option<gtk::Widget> {
    if widget.widget_name() == id {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = find(&widget, id) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
#[ignore = "requires X11 display and its own GTK initialization process"]
fn native_settings_layout_is_disjoint_and_scrolls_when_short() {
    gtk::init().unwrap();
    let host = panechord_gtk::GtkHost::attach();
    let model = Store::new(
        crate::model::App::new(crate::persistence::Config::default()),
        host.executor(),
    );
    let view = panechord_macros::view!("ui/settings.yaml", model, backend);
    let window = view.native_window();
    view.present();
    pump_until(|| window.is_mapped());
    let ids = [
        "heading",
        "name_caption",
        "name",
        "volume_caption",
        "volume",
        "scale_caption",
        "scale",
        "error",
        "summary",
        "status",
        "save",
        "cancel",
        "warning",
        "save_exit",
        "discard_exit",
        "keep_editing",
    ];
    let window_widget = window.upcast_ref::<gtk::Widget>();
    let screen = find(window_widget, "screen").unwrap();
    let mut ancestor = screen.parent().unwrap();
    let scroll = loop {
        if let Ok(scroll) = ancestor.clone().downcast::<gtk::ScrolledWindow>() {
            break scroll;
        }
        ancestor = ancestor.parent().unwrap();
    };
    for (state, message) in [
        ("initial", ""),
        ("invalid", "Volume must be an integer from 0 to 100"),
        (
            "confirm",
            "Unsaved changes. Save and exit, discard, or keep editing.",
        ),
        (
            "long",
            "A very long validation message with an_unbroken_word_that_must_not_escape_the_window_bounds and multiple lines of detail. A very long validation message with additional detail.",
        ),
    ] {
        assert!(model.update(move |state| {
            state.ui.error = message.into();
            state.ui.warning = message.into();
        }));
        for width in [440, 660, 960] {
            window.set_default_size(width, 360);
            scroll.vadjustment().set_value(0.0);
            pump_until(|| window.width() == width);
            let ticked = std::rc::Rc::new(std::cell::Cell::new(false));
            let observed = ticked.clone();
            window.add_tick_callback(move |_, _| {
                observed.set(true);
                gtk::glib::ControlFlow::Break
            });
            pump_until(|| ticked.get());
            let outer = screen.compute_bounds(window_widget).unwrap();
            let bounds: Vec<_> = ids
                .iter()
                .map(|id| {
                    let frame = find(window_widget, id).unwrap();
                    if matches!(
                        *id,
                        "save" | "cancel" | "save_exit" | "discard_exit" | "keep_editing"
                    ) {
                        let button = frame.first_child().unwrap().compute_bounds(&frame).unwrap();
                        assert_eq!(
                            frame.width() as f32,
                            button.width(),
                            "button must fill frame: {id}"
                        );
                    }
                    (*id, frame.compute_bounds(window_widget).unwrap())
                })
                .collect();
            for (id, rect) in &bounds {
                assert!(
                    rect.x() >= outer.x() + 19.0
                        && rect.x() + rect.width() <= outer.x() + outer.width() - 19.0,
                    "{state}/{width}: {id}: {rect:?}"
                );
                assert!(
                    rect.y() >= outer.y() + 19.0
                        && rect.y() + rect.height() <= outer.y() + outer.height() - 19.0,
                    "{state}/{width}: {id} exceeds content height"
                );
            }
            for (index, (id, rect)) in bounds.iter().enumerate() {
                for (other, other_rect) in &bounds[index + 1..] {
                    assert!(
                        rect.intersection(other_rect).is_none(),
                        "{state}/{width}: {id} overlaps {other}"
                    );
                }
            }
            let adjustment = scroll.vadjustment();
            assert!(adjustment.upper() > adjustment.page_size());
            adjustment.set_value(adjustment.upper() - adjustment.page_size());
            pump_until(|| {
                let last = find(window_widget, "keep_editing")
                    .unwrap()
                    .compute_bounds(window_widget)
                    .unwrap();
                let viewport = scroll.compute_bounds(window_widget).unwrap();
                last.y() + last.height() <= viewport.y() + viewport.height()
            });
        }
    }
    view.close();
    host.close();
}

#[test]
#[ignore = "requires X11 display and its own GTK initialization process"]
fn native_generated_shared_windows_survive_peer_close() {
    gtk::init().unwrap();
    let application = gtk::Application::builder()
        .application_id("org.nerust.PanechordPrototype3Shared")
        .build();
    application
        .register(None::<&gtk::gio::Cancellable>)
        .unwrap();
    let host = panechord_gtk::GtkHost::attach();
    let model = Store::new(
        crate::model::App::new(crate::persistence::Config::default()),
        host.executor(),
    );
    let first = panechord_macros::view!("ui/settings.yaml", model, backend);
    let second = panechord_macros::view!("ui/settings.yaml", model, backend);
    for view in [&first, &second] {
        view.native_window().set_application(Some(&application));
        view.present();
    }
    pump_until(|| first.native_window().is_mapped() && second.native_window().is_mapped());
    first.native_window().close();
    assert!(first.downgrade().upgrade().is_none());
    assert!(!model.is_closed());
    assert!(second.downgrade().upgrade().is_some());
    second.edit("name", "second survives").unwrap();
    assert_eq!(model.snapshot().draft.name, "second survives");
    assert!(model.effect(
        Policy::Parallel,
        "peer-close",
        async { String::from("async result after peer close") },
        |state, name| state.draft.name = name,
    ));
    pump_until(|| second.text("name").as_deref() == Some("async result after peer close"));
    let final_binding = model.bind(
        |s| s.draft.name.clone(),
        Some(|s, value| s.draft.name = value),
    );
    second.native_window().close();
    assert!(!model.is_closed(), "the explicit model owner still exists");
    assert!(final_binding.set("retained by explicit owner".into()));
    drop(model);
    assert!(!final_binding.set("no owners".into()));
    assert_eq!(final_binding.get(), "retained by explicit owner");
    host.close();
}
