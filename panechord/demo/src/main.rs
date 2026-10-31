//! Application setup only; behavior, extension primitives and probes are separate.
mod model;
mod native;
mod smoke;

#[cfg(feature = "backend-gtk")]
use panechord_gtk as backend;
#[cfg(all(feature = "backend-tao", not(feature = "backend-gtk")))]
use panechord_tao as backend;
#[cfg(any(
    all(feature = "backend-gtk", feature = "backend-tao"),
    not(any(feature = "backend-gtk", feature = "backend-tao"))
))]
compile_error!("select exactly one backend");

use model::{RowCommands, Settings, SettingsCommands};
use native::{NumberText, meter, validate_name};
use panechord_runtime::{BlockingPool, Store};
use std::{cell::RefCell, rc::Rc};

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg != "--smoke") {
        return Err(
            "usage: panechord-demo [--smoke] (minimal example: cargo run --example minimal)".into(),
        );
    }
    let workers = Rc::new(RefCell::new(BlockingPool::new(2, 8)));
    let pool = workers.clone();
    let result = backend::run(move |executor| {
        let path = std::env::temp_dir().join(format!("panechord-demo-{}.txt", std::process::id()));
        let model = Store::new(
            Settings::new(executor.clone(), pool, path.clone()),
            executor.clone(),
        );
        let owner = model
            .try_into_owner()
            .unwrap_or_else(|_| panic!("shared model"));
        let model = owner.store().clone();
        let view = panechord_macros::owned_view!("ui/gallery.yaml", owner, backend);
        if args.iter().any(|arg| arg == "--smoke") {
            executor.spawn(Box::pin(smoke::verify(view.clone(), model, path)));
        }
        view
    });
    // Blocking joins never happen inside the live native loop.
    workers.borrow_mut().shutdown();
    result
}
