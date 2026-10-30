//! Automated verification is not application boilerplate.
use crate::{backend, model::Settings};
use std::{cell::RefCell, path::PathBuf, rc::Rc};
use wakui2_runtime::{Binding, Store};

async fn until<T: Clone + 'static>(binding: Binding<T>, predicate: impl Fn(&T) -> bool + 'static) {
    let (sender, receiver) = futures::channel::oneshot::channel();
    let sender = RefCell::new(Some(sender));
    let token = binding.subscribe(Rc::new(move |value| {
        if predicate(&value)
            && let Some(sender) = sender.borrow_mut().take()
        {
            let _ = sender.send(());
        }
    }));
    receiver.await.unwrap();
    drop(token);
}
pub async fn verify(view: backend::View, model: Store<Settings>, path: PathBuf) {
    until(model.bind(|s| s.validating, None), |pending| !pending).await;
    assert_eq!(view.item_keys("rows"), ["0", "1", "2", "3"]);
    assert!(view.mounted_count("rows") <= 5);
    view.edit("name", "Ada").unwrap();
    assert_eq!(view.text("echo").as_deref(), Some("Ada"));
    assert!(view.edit("readonly", "not writable").is_err());
    assert!(model.snapshot().validating);
    until(model.bind(|s| s.validating, None), |pending| !pending).await;
    view.edit("number", "0.75").unwrap();
    assert_eq!(model.snapshot().progress, 0.75);
    // Both adapters use the model as canonical value after a rejected conversion.
    let _ = view.edit("number", "not a number");
    assert_eq!(model.snapshot().progress, 0.75);
    view.edit("rows/0/name", "edited item").unwrap();
    view.click("rows/0/rename").unwrap();
    assert_eq!(
        model.snapshot().rows[0].model.snapshot().name,
        "EDITED ITEM"
    );
    view.viewport("rows", 500);
    assert!(view.mounted_count("rows") <= 5);
    assert!(view.text("rows/0/name").is_none());
    view.viewport("rows", 0);
    assert_eq!(view.text("rows/0/name").as_deref(), Some("EDITED ITEM"));
    assert_eq!(view.text("selected/0/name").as_deref(), Some("EDITED ITEM"));
    view.activate_menu("actions/save").unwrap();
    assert!(model.snapshot().busy);
    assert!(view.click("save").is_err());
    until(model.bind(|s| s.busy, None), |busy| !busy).await;
    assert_eq!(model.snapshot().saved, 1, "{}", model.snapshot().status);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Ada");
    view.edit("name", "").unwrap();
    until(model.bind(|s| s.validating, None), |pending| !pending).await;
    assert_eq!(view.text("error").as_deref(), Some("A name is required"));
    assert!(view.click("save").is_err());
    view.edit("name", "Grace").unwrap();
    until(model.bind(|s| s.validating, None), |pending| !pending).await;
    view.click("save").unwrap();
    until(model.bind(|s| s.busy, None), |busy| !busy).await;
    assert_eq!(model.snapshot().saved, 2);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Grace");
    view.activate_menu("actions/reverse").unwrap();
    assert_eq!(
        view.item_keys("rows").first().map(String::as_str),
        Some("999")
    );
    assert_eq!(view.text("selected/999/name").as_deref(), Some("Item 999"));
    view.close();
    view.close();
    assert!(model.is_closed());
    let _ = std::fs::remove_file(path);
    println!(
        "PROTOTYPE2 PASS templates=item+content virtualization=1000 conversions=checked validation=async menu=dynamic saves=2 closed=true"
    );
}
