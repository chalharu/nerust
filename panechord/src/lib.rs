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
pub use panechord_gtk::{Parent, View, WeakView, pick_file};
#[cfg(feature = "macos")]
pub use panechord_macos as macos;
#[cfg(feature = "macos")]
pub use panechord_macos::{Parent, View, WeakView, pick_file};
#[cfg(feature = "tao")]
pub use panechord_tao as tao;
#[cfg(feature = "tao")]
pub use panechord_tao::{Parent, View, WeakView, pick_file};

#[cfg(all(
    feature = "native",
    not(any(feature = "gtk", feature = "tao", feature = "macos"))
))]
pub use panechord_native::{Parent, View, WeakView, pick_file};

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
        RunnerBuilder {}
    }
}
#[cfg(any(
    feature = "native",
    feature = "gtk",
    feature = "tao",
    feature = "macos"
))]
pub struct RunnerBuilder {}
#[cfg(any(
    feature = "native",
    feature = "gtk",
    feature = "tao",
    feature = "macos"
))]
impl RunnerBuilder {
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
            panechord_native::run_host(move |host| make(host, &config))
        }
        #[cfg(feature = "tao")]
        {
            let config = panechord_tao::runner_config();
            panechord_tao::run_host(move |host| make(host, &config))
        }
        #[cfg(feature = "gtk")]
        {
            let config = panechord_gtk::runner_config();
            panechord_gtk::run_host(move |host| make(host, &config))
        }
        #[cfg(feature = "macos")]
        {
            let config = panechord_macos::runner_config();
            panechord_macos::run_host(move |host| make(host, &config))
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
