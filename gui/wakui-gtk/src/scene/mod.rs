//! Scene realization: the [`wakui_core::scene`] vocabulary made native.
//! [`binding`] hosts one retained binding per control screen (snapshot,
//! queue, mailbox); [`render`] interprets screens as GTK widgets.
pub mod binding;
pub mod render;

#[cfg(test)]
#[path = "controls_tests.rs"]
mod controls_tests;
