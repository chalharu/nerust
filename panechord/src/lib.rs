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
pub use panechord_gtk::{Parent, View, WeakView, pick_file, set_allows_automatic_window_tabbing, set_initial_window_size};
#[cfg(feature = "macos")]
pub use panechord_macos as macos;
#[cfg(feature = "macos")]
pub use panechord_macos::{Parent, View, WeakView, pick_file, set_allows_automatic_window_tabbing, set_initial_window_size};
#[cfg(feature = "tao")]
pub use panechord_tao as tao;
#[cfg(feature = "tao")]
pub use panechord_tao::{Parent, View, WeakView, pick_file, set_allows_automatic_window_tabbing, set_initial_window_size};

#[cfg(all(
    feature = "native",
    not(any(feature = "gtk", feature = "tao", feature = "macos"))
))]
pub use panechord_native::{Parent, View, WeakView, pick_file, set_allows_automatic_window_tabbing, set_initial_window_size};

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
/// macOS-only builder options, mirroring tao's `WindowBuilderExtMacOS`.
/// Available only on Apple targets; other platforms have no window tabbing.
#[cfg(all(
    target_os = "macos",
    any(feature = "native", feature = "gtk", feature = "tao", feature = "macos")
))]
pub trait RunnerBuilderExtMacOS: Sized {
    /// Opt out of (or back into) AppKit automatic window tabbing. Takes
    /// effect for windows created after `run`. When unset, each backend
    /// keeps its default (the AppKit host opts out; tao/gtk keep the
    /// system default).
    fn allows_automatic_window_tabbing(self, enabled: bool) -> Self;
}

#[cfg(all(
    target_os = "macos",
    any(feature = "native", feature = "gtk", feature = "tao", feature = "macos")
))]
impl RunnerBuilderExtMacOS for RunnerBuilder {
    fn allows_automatic_window_tabbing(mut self, enabled: bool) -> Self {
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
        #[cfg(all(
            feature = "native",
            not(any(feature = "gtk", feature = "tao", feature = "macos"))
        ))]
        {
            let config = panechord_native::runner_config();
            let tabbing = self.automatic_window_tabbing;
            let size = self.initial_window_size;
            panechord_native::run_host(move |host| {
                if let Some(enabled) = tabbing {
                    panechord_native::set_allows_automatic_window_tabbing(enabled);
                }
                if let Some((width, height)) = size {
                    panechord_native::set_initial_window_size(width, height);
                }
                make(host, &config)
            })
        }
        #[cfg(feature = "tao")]
        {
            let config = panechord_tao::runner_config();
            let tabbing = self.automatic_window_tabbing;
            let size = self.initial_window_size;
            panechord_tao::run_host(move |host| {
                if let Some(enabled) = tabbing {
                    panechord_tao::set_allows_automatic_window_tabbing(enabled);
                }
                if let Some((width, height)) = size {
                    panechord_tao::set_initial_window_size(width, height);
                }
                make(host, &config)
            })
        }
        #[cfg(feature = "gtk")]
        {
            let config = panechord_gtk::runner_config();
            let tabbing = self.automatic_window_tabbing;
            let size = self.initial_window_size;
            panechord_gtk::run_host(move |host| {
                if let Some(enabled) = tabbing {
                    panechord_gtk::set_allows_automatic_window_tabbing(enabled);
                }
                if let Some((width, height)) = size {
                    panechord_gtk::set_initial_window_size(width, height);
                }
                make(host, &config)
            })
        }
        #[cfg(feature = "macos")]
        {
            let config = panechord_macos::runner_config();
            let tabbing = self.automatic_window_tabbing;
            let size = self.initial_window_size;
            panechord_macos::run_host(move |host| {
                if let Some(enabled) = tabbing {
                    panechord_macos::set_allows_automatic_window_tabbing(enabled);
                }
                if let Some((width, height)) = size {
                    panechord_macos::set_initial_window_size(width, height);
                }
                make(host, &config)
            })
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
