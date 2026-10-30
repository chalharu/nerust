//! Exact/ignored execution in a fresh GTK process, never alongside GTK3/Tao.
use crate::{backend, model::AppCommands};
use std::time::Duration;
use wakui2_gtk::gtk::{self, prelude::*};
use wakui2_runtime::{Policy, Store};

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

#[test]
#[ignore = "requires X11 display and its own GTK initialization process"]
fn native_generated_shared_windows_survive_peer_close() {
    gtk::init().unwrap();
    let application = gtk::Application::builder()
        .application_id("org.nerust.WakuiPrototype3Shared")
        .build();
    application
        .register(None::<&gtk::gio::Cancellable>)
        .unwrap();
    let host = wakui2_gtk::GtkHost::attach();
    let model = Store::new(
        crate::model::App::new(crate::persistence::Config::default()),
        host.executor(),
    );
    let first = wakui2_macros::view!("ui/settings.yaml", model, backend);
    let second = wakui2_macros::view!("ui/settings.yaml", model, backend);
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
