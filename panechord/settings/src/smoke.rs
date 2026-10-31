//! Real adapter operations; binding-driven waits have a hard deadline, no sleeps.
use crate::{
    backend,
    model::{App, Draft, Exit},
    persistence,
};
use futures::{
    channel::oneshot,
    future::{Either, select},
};
use panechord_runtime::{Binding, Store};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

pub async fn deadline<F: std::future::Future + Unpin>(future: F) -> F::Output {
    match select(future, futures_timer::Delay::new(Duration::from_secs(8))).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => panic!("settings async deadline exceeded"),
    }
}

pub async fn until<T: Clone + 'static>(
    binding: Binding<T>,
    predicate: impl Fn(&T) -> bool + 'static,
) {
    let (sender, receiver) = oneshot::channel();
    let sender = RefCell::new(Some(sender));
    let token = binding.subscribe(Rc::new(move |value| {
        if predicate(&value)
            && let Some(sender) = sender.borrow_mut().take()
        {
            let _ = sender.send(());
        }
    }));
    deadline(receiver)
        .await
        .expect("binding subscription was canceled");
    drop(token);
}

pub async fn verify(view: backend::View, model: Store<App>, path: PathBuf, passed: Rc<Cell<bool>>) {
    let initial = model.snapshot().saved;
    assert_eq!(view.text("name").as_deref(), Some(initial.name.as_str()));
    assert!(view.text("summary").unwrap().contains("Saved: Player"));
    assert!(view.edit("summary", "cannot change saved config").is_err());
    assert!(view.click("save_exit").is_err());
    assert!(view.click("discard_exit").is_err());
    assert!(view.click("keep_editing").is_err());
    for invalid in ["", "-", "abc"] {
        view.edit("volume", invalid).unwrap();
        assert_eq!(view.text("volume").as_deref(), Some(invalid));
        assert_eq!(model.snapshot().draft.volume, invalid);
        assert_eq!(model.snapshot().saved, initial);
        assert!(!view.text("error").unwrap().is_empty());
        assert!(view.click("save").is_err());
        assert!(view.activate_menu("file/save").is_err());
    }
    view.click("cancel").unwrap();
    assert_eq!(model.snapshot().draft, Draft::from(&initial));
    assert!(view.text("error").unwrap().is_empty());
    assert!(!path.exists(), "Cancel must not write");
    println!("SETTINGS3 PASS entries readonly invalid-retention save-gating cancel-no-write");

    view.edit("name", "Ada").unwrap();
    view.activate_menu("file/save").unwrap();
    assert!(model.snapshot().busy());
    assert!(view.click("save").is_err());
    assert!(view.click("cancel").is_err());
    until(model.bind(App::busy, None), |busy| !busy).await;
    assert_eq!(model.snapshot().saved.name, "Ada");
    assert_eq!(persistence::load(&path).unwrap().name, "Ada");
    assert!(view.text("summary").unwrap().contains("Ada"));
    let bytes = std::fs::read(&path).unwrap();

    view.edit("name", "WM edit").unwrap();
    view.request_close();
    assert_eq!(model.snapshot().exit, Exit::Confirm);
    assert!(!view.text("warning").unwrap().is_empty());
    view.click("keep_editing").unwrap();
    assert_eq!(model.snapshot().draft.name, "WM edit");
    assert!(!model.is_closed());
    view.activate_menu("file/quit").unwrap();
    assert_eq!(model.snapshot().exit, Exit::Confirm);
    view.click("cancel").unwrap();
    assert_eq!(model.snapshot().saved.name, "Ada");
    assert_eq!(model.snapshot().draft.name, "Ada");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    println!("SETTINGS3 PASS async-save wm-close-veto menu-quit-veto keep-editing cancel-close");

    view.edit("name", "Captured").unwrap();
    view.click("save").unwrap();
    view.request_close();
    assert_eq!(model.snapshot().exit, Exit::Waiting);
    // Still in this executor turn, before the save completion can be applied.
    view.edit("volume", "-").unwrap();
    assert!(view.click("discard_exit").is_err());
    until(model.bind(App::busy, None), |busy| !busy).await;
    assert_eq!(model.snapshot().exit, Exit::Confirm);
    assert_eq!(view.text("volume").as_deref(), Some("-"));
    assert_eq!(model.snapshot().saved.name, "Captured");
    assert!(view.click("save_exit").is_err());
    view.click("keep_editing").unwrap();
    view.click("cancel").unwrap();
    println!("SETTINGS3 PASS busy-exit-waits newer-invalid-edit-reprompts");

    view.edit("name", "Grace").unwrap();
    view.edit("volume", "42").unwrap();
    view.edit("scale", "1.5").unwrap();
    view.activate_menu("file/quit").unwrap();
    assert_eq!(model.snapshot().exit, Exit::Confirm);
    // The final save closes the native executor. Check/print from the close
    // callback, not a future that might be canceled while awaiting that close.
    let observer = model.clone();
    view.on_close(Rc::new(move || {
        assert!(observer.is_closed());
        let state = observer.snapshot();
        assert!(state.close_ready());
        assert!(!state.dirty());
        assert_eq!(state.saved.name, "Grace");
        assert_eq!(state.saved.volume, 42);
        assert_eq!(state.saved.scale, 1.5);
        assert_eq!(persistence::load(&path).unwrap(), state.saved);
        passed.set(true);
        println!("SETTINGS3 PASS final-save-exit saved-json verified model-closed");
    }));
    view.click("save_exit").unwrap();
}
