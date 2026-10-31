//! Only CLI, startup load and native-loop ownership live here.
mod model;
mod persistence;
mod smoke;
mod wiring;

#[cfg(all(test, feature = "backend-gtk"))]
mod native_tests;

#[cfg(feature = "backend-gtk")]
use panechord_gtk as backend;
#[cfg(all(
    feature = "backend-macos",
    not(any(feature = "backend-gtk", feature = "backend-tao"))
))]
use panechord_macos as backend;
#[cfg(all(feature = "backend-tao", not(feature = "backend-gtk")))]
use panechord_tao as backend;
#[cfg(any(
    all(feature = "backend-gtk", feature = "backend-tao"),
    all(
        feature = "backend-macos",
        any(feature = "backend-gtk", feature = "backend-tao")
    ),
    not(any(
        feature = "backend-gtk",
        feature = "backend-tao",
        feature = "backend-macos"
    ))
))]
compile_error!("select exactly one backend: backend-gtk, backend-tao or backend-macos");

use panechord_runtime::{BlockingPool, Store};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

fn main() -> Result<(), String> {
    let (path, smoke) = options(std::env::args_os().skip(1))?;
    let sandbox = if smoke {
        Some(persistence::Sandbox::new()?)
    } else {
        None
    };
    let path = sandbox.as_ref().map_or(path, persistence::Sandbox::path);
    // Invalid existing data is fatal BEFORE starting a native loop or any worker.
    let config = persistence::load(&path)?;
    let workers = Rc::new(RefCell::new(BlockingPool::new(1, 4)));
    let pool = workers.clone();
    let passed = Rc::new(std::cell::Cell::new(false));
    let probe = passed.clone();
    let result = backend::run(move |executor| {
        #[cfg(not(feature = "backend-macos"))]
        let model = Store::new(model::App::new(config), executor.clone());
        #[cfg(feature = "backend-macos")]
        let model = Store::with_dispatch_budget(
            model::App::new(config),
            executor.clone(),
            panechord_runtime::Limits::default(),
            std::num::NonZeroUsize::new(64).unwrap(),
        );
        let view = wiring::view(model.clone(), pool, path.clone());
        if smoke {
            executor.spawn(Box::pin(smoke::verify(view.clone(), model, path, probe)));
        }
        view
    });
    workers.borrow_mut().shutdown(); // No blocking joins inside the UI loop.
    result?;
    if smoke && !passed.get() {
        return Err("settings smoke did not finish".into());
    }
    drop(sandbox);
    Ok(())
}

fn options(args: impl Iterator<Item = std::ffi::OsString>) -> Result<(PathBuf, bool), String> {
    let mut args = args;
    let mut path = None;
    let mut smoke = false;
    while let Some(arg) = args.next() {
        if arg == "--smoke" && !smoke {
            smoke = true;
        } else if arg == "--config" && path.is_none() {
            path = Some(PathBuf::from(
                args.next().ok_or("--config requires a path")?,
            ));
        } else {
            return Err("usage: panechord-settings [--config PATH] [--smoke]".into());
        }
    }
    let path = match path {
        Some(path) => path,
        None => std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join("panechord-settings.json"),
    };
    Ok((path, smoke))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<(PathBuf, bool), String> {
        options(args.iter().map(std::ffi::OsString::from))
    }

    #[test]
    fn cli_default_and_optional_config() {
        assert_eq!(
            parse(&[]).unwrap(),
            (
                std::env::current_dir()
                    .unwrap()
                    .join("panechord-settings.json"),
                false
            )
        );
        assert_eq!(
            parse(&["--config", "my config.json"]).unwrap(),
            ("my config.json".into(), false)
        );
        assert_eq!(
            parse(&["--smoke", "--config", "untouched.json"]).unwrap(),
            ("untouched.json".into(), true)
        );
    }

    #[test]
    fn cli_rejects_missing_duplicate_and_unknown_arguments() {
        for args in [
            vec!["--config"],
            vec!["--smoke", "--smoke"],
            vec!["--unknown"],
            vec!["--config", "a", "--config", "b"],
        ] {
            assert!(parse(&args).is_err());
        }
    }
}
