//! Application setup only; behavior, extension primitives and probes are separate.
mod model;
mod native;
mod smoke;

#[cfg(feature = "backend-gtk")]
use wakui2_gtk as backend;
#[cfg(all(feature = "backend-tao", not(feature = "backend-gtk")))]
use wakui2_tao as backend;
#[cfg(any(
    all(feature = "backend-gtk", feature = "backend-tao"),
    not(any(feature = "backend-gtk", feature = "backend-tao"))
))]
compile_error!("select exactly one backend");

use model::{RowCommands, Settings, SettingsCommands};
use native::{NumberText, meter, validate_name};
use std::{cell::RefCell, rc::Rc};
use wakui2_runtime::{BlockingPool, Store};

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg != "--smoke") {
        return Err(
            "usage: wakui2-demo [--smoke] (minimal example: cargo run --example minimal)".into(),
        );
    }
    let workers = Rc::new(RefCell::new(BlockingPool::new(2, 8)));
    let pool = workers.clone();
    let result = backend::run(move |executor| {
        let path = std::env::temp_dir().join(format!("wakui2-demo-{}.txt", std::process::id()));
        let model = Store::new(
            Settings::new(executor.clone(), pool, path.clone()),
            executor.clone(),
        );
        let view = wakui2_macros::owned_view!("ui/gallery.yaml", model, backend);
        if args.iter().any(|arg| arg == "--smoke") {
            executor.spawn(Box::pin(smoke::verify(view.clone(), model, path)));
        }
        view
    });
    // Blocking joins never happen inside the live native loop.
    workers.borrow_mut().shutdown();
    result
}
