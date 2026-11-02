pub use panechord_macros::{commands, owned_view, try_owned_view, try_view, view};
pub use panechord_runtime::*;

#[cfg(any(
    all(feature = "gtk", feature = "tao"),
    all(feature = "gtk", feature = "macos"),
    all(feature = "tao", feature = "macos")
))]
compile_error!("select exactly one Panechord native backend");

#[cfg(feature = "gtk")]
pub use panechord_gtk as gtk;
#[cfg(feature = "gtk")]
pub use panechord_gtk::{
    Parent, View, WeakView, pick_file, set_allows_automatic_window_tabbing, set_initial_window_size,
};
#[cfg(feature = "macos")]
pub use panechord_macos as macos;
#[cfg(feature = "macos")]
pub use panechord_macos::{
    Parent, View, WeakView, pick_file, set_allows_automatic_window_tabbing, set_initial_window_size,
};
#[cfg(feature = "tao")]
pub use panechord_tao as tao;
#[cfg(feature = "tao")]
pub use panechord_tao::{
    Parent, View, WeakView, pick_file, set_allows_automatic_window_tabbing, set_initial_window_size,
};

#[cfg(all(
    feature = "native",
    not(any(feature = "gtk", feature = "tao", feature = "macos"))
))]
pub use panechord_native::{
    Parent, View, WeakView, pick_file, set_allows_automatic_window_tabbing, set_initial_window_size,
};

/// Startup builder: backend-resolved run configuration flows into the
/// application instead of views querying backend capabilities, so every
/// branch stays unit-testable on any platform.
#[cfg(any(
    feature = "native",
    feature = "gtk",
    feature = "tao",
    feature = "macos"
))]
pub struct Runner;
#[cfg(any(
    feature = "native",
    feature = "gtk",
    feature = "tao",
    feature = "macos"
))]
impl Runner {
    pub fn builder() -> RunnerBuilder {
        RunnerBuilder {
            automatic_window_tabbing: None,
            initial_window_size: None,
        }
    }
}
#[cfg(any(
    feature = "native",
    feature = "gtk",
    feature = "tao",
    feature = "macos"
))]
pub struct RunnerBuilder {
    automatic_window_tabbing: Option<bool>,
    initial_window_size: Option<(u32, u32)>,
}
/// Whether the backend may merge separate windows into the platform's
/// automatic window-tab bar. This is a portable intent, not a macOS API:
/// only platforms that merge windows into tabs (macOS AppKit) honor it;
/// everywhere else it is a recorded no-op, so frontend crates stay free
/// of `target_os` branches. When unset, each backend keeps its default.
#[cfg(any(
    feature = "native",
    feature = "gtk",
    feature = "tao",
    feature = "macos"
))]
impl RunnerBuilder {
    pub fn automatic_window_tabbing(mut self, enabled: bool) -> Self {
        self.automatic_window_tabbing = Some(enabled);
        self
    }
}

#[cfg(any(
    feature = "native",
    feature = "gtk",
    feature = "tao",
    feature = "macos"
))]
impl RunnerBuilder {
    /// Restore the remembered main-window size (logical points) for the
    /// first window `run` builds. Out-of-range values keep the default.
    pub fn initial_window_size(mut self, width: u32, height: u32) -> Self {
        self.initial_window_size = Some((width, height));
        self
    }

    pub fn run(
        self,
        make: impl FnOnce(std::rc::Rc<dyn WindowHost<View>>, &RunnerConfig) -> Result<(), String>
        + 'static,
    ) -> Result<(), String> {
        // One dispatch shape for every backend: each exposes `runner_config`,
        // `run_host` and the pre-build option setters. A new builder option
        // only touches this macro; a new backend only adds one arm below.
        macro_rules! run_with_backend {
            ($backend:ident) => {{
                let config = $backend::runner_config();
                let tabbing = self.automatic_window_tabbing;
                let size = self.initial_window_size;
                $backend::run_host(move |host| {
                    if let Some(enabled) = tabbing {
                        $backend::set_allows_automatic_window_tabbing(enabled);
                    }
                    if let Some((width, height)) = size {
                        $backend::set_initial_window_size(width, height);
                    }
                    make(host, &config)
                })
            }};
        }
        #[cfg(all(
            feature = "native",
            not(any(feature = "gtk", feature = "tao", feature = "macos"))
        ))]
        {
            run_with_backend!(panechord_native)
        }
        #[cfg(feature = "tao")]
        {
            run_with_backend!(panechord_tao)
        }
        #[cfg(feature = "gtk")]
        {
            run_with_backend!(panechord_gtk)
        }
        #[cfg(feature = "macos")]
        {
            run_with_backend!(panechord_macos)
        }
        #[cfg(not(any(
            feature = "native",
            feature = "gtk",
            feature = "tao",
            feature = "macos"
        )))]
        {
            let _ = make;
            Err("no Panechord native backend selected".into())
        }
    }
}
