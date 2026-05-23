//! Re-exports from [`nerust_console`] surfaced here so that
//! `nerust_gui_shell` can avoid a direct dependency on that crate.
//! Keep this module narrow; if more than a few types are needed,
//! prefer a direct dependency in the consumer instead.
pub use nerust_console::Console;
