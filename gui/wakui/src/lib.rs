#[cfg(all(
    feature = "wakui_default",
    not(feature = "wakui_gtk"),
    not(feature = "wakui_tao")
))]
pub use wakui_default::{run, show_message_dialog};

#[cfg(all(feature = "wakui_gtk", not(feature = "wakui_tao")))]
pub use wakui_gtk::{run, show_message_dialog};

#[cfg(all(feature = "wakui_tao", not(feature = "wakui_gtk")))]
pub use wakui_tao::{run, show_message_dialog};

#[cfg(feature = "wakui_gtk")]
pub mod gtk {
    pub use wakui_gtk::*;
}

#[cfg(feature = "wakui_tao")]
pub mod tao {
    pub use wakui_tao::*;
}

pub use wakui_core::*;
