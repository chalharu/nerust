//! Smoke fixture writes are isolated from the read-only scanner and user folders.
use crate::{backend, model::App};
use futures::{
    channel::oneshot,
    future::{Either, select},
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use wakui2_runtime::{Binding, Store};

static NEXT: AtomicU64 = AtomicU64::new(0);
pub struct Sandbox {
    pub dir: PathBuf,
}
impl Sandbox {
    pub fn new() -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!(
            "wakui3-library-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // Never adopt or remove an existing directory, even on a name collision.
        std::fs::create_dir(&dir).map_err(|error| error.to_string())?;
        Ok(Self { dir })
    }
    pub fn populate(&self) -> Result<(), String> {
        for name in [
            "Alpha.nes",
            "Beta.gb",
            "Gamma.GBC",
            "Delta.gba",
            "ignored.txt",
        ] {
            std::fs::write(self.dir.join(name), b"fixture, not a ROM")
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
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
    match select(receiver, futures_timer::Delay::new(Duration::from_secs(8))).await {
        Either::Left((result, _)) => result.expect("library subscription canceled"),
        Either::Right(_) => panic!("LIBRARY3 async deadline exceeded"),
    }
    drop(token);
}

pub async fn verify(view: backend::View, model: Store<App>, passed: Rc<Cell<bool>>) {
    #[cfg(feature = "backend-gtk")]
    let name = "gtk";
    #[cfg(all(feature = "backend-tao", not(feature = "backend-gtk")))]
    let name = "tao";
    until(
        model.bind(|s| !s.busy && s.rows.len() == 4, None),
        |ready| *ready,
    )
    .await;
    assert_eq!(view.item_keys("rows").len(), 4);
    assert!(view.text("details").unwrap().contains("Alpha.nes"));
    view.click("next").unwrap();
    assert!(view.text("details").unwrap().contains("Beta.gb"));
    view.edit("filter", "GAMMA").unwrap();
    until(
        model.bind(|s| !s.busy && s.rows.len() == 1, None),
        |ready| *ready,
    )
    .await;
    assert!(view.text("details").unwrap().contains("Gamma.GBC"));
    assert_eq!(view.item_keys("rows").len(), 1);
    view.edit("filter", "").unwrap();
    until(
        model.bind(|s| !s.busy && s.rows.len() == 4, None),
        |ready| *ready,
    )
    .await;
    let folder = model.snapshot().folder;
    view.edit("folder", &format!("{folder}/missing")).unwrap();
    until(
        model.bind(|s| !s.busy && !s.error.is_empty(), None),
        |ready| *ready,
    )
    .await;
    assert!(!view.text("error").unwrap().is_empty());
    view.edit("folder", &folder).unwrap();
    until(
        model.bind(|s| !s.busy && s.rows.len() == 4, None),
        |ready| *ready,
    )
    .await;
    view.click("rescan").unwrap();
    until(
        model.bind(|s| !s.busy && s.rows.len() == 4, None),
        |ready| *ready,
    )
    .await;
    println!(
        "LIBRARY3 {name} PASS native-events scan filter selection invalid-folder recovery rescan"
    );
    let owner = model.clone();
    view.on_close(Rc::new(move || {
        assert!(owner.is_closed());
        passed.set(true);
        println!("LIBRARY3 {name} PASS native-close model-closed");
    }));
    view.request_close();
}
