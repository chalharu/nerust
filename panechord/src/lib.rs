pub use panechord_macros::{commands, owned_view, try_owned_view, try_view, view};
pub use panechord_runtime::*;

#[cfg(any(
    all(feature = "gtk", feature = "tao"),
    all(feature = "gtk", feature = "macos"),
    all(feature = "tao", feature = "macos")
))]
compile_error!("select exactly one Panechord native backend");

#[cfg(any(
    feature = "native",
    feature = "gtk",
    feature = "tao",
    feature = "macos"
))]
pub fn run_host(
    make: impl FnOnce(std::rc::Rc<dyn WindowHost<View>>) -> Result<(), String> + 'static,
) -> Result<(), String> {
    #[cfg(all(
        feature = "native",
        not(any(feature = "gtk", feature = "tao", feature = "macos"))
    ))]
    {
        panechord_native::run_host(move |host| make(host))
    }
    #[cfg(feature = "tao")]
    {
        panechord_tao::run_host(move |host| make(host))
    }
    #[cfg(feature = "gtk")]
    {
        panechord_gtk::run_host(move |host| make(host))
    }
    #[cfg(feature = "macos")]
    {
        panechord_macos::run_host(move |host| make(host))
    }
}

#[cfg(feature = "gtk")]
pub use panechord_gtk as gtk;
#[cfg(feature = "gtk")]
pub use panechord_gtk::wants_application_menu;
#[cfg(feature = "gtk")]
pub use panechord_gtk::{Parent, View, WeakView};
#[cfg(feature = "macos")]
pub use panechord_macos as macos;
#[cfg(feature = "macos")]
pub use panechord_macos::wants_application_menu;
#[cfg(feature = "macos")]
pub use panechord_macos::{Parent, View, WeakView};
#[cfg(all(
    feature = "native",
    not(any(feature = "gtk", feature = "tao", feature = "macos"))
))]
pub use panechord_native::wants_application_menu;
#[cfg(feature = "tao")]
pub use panechord_tao as tao;
#[cfg(feature = "tao")]
pub use panechord_tao::wants_application_menu;
#[cfg(feature = "tao")]
pub use panechord_tao::{Parent, View, WeakView};

#[cfg(all(
    feature = "native",
    not(any(feature = "gtk", feature = "tao", feature = "macos"))
))]
pub use panechord_native::{Parent, View, WeakView};
