//! Scene realization: the [`wakui_core::scene`] vocabulary rendered and bound.
//! [`render`] interprets control screens as iced elements; [`binding`] hosts
//! one live binding per control session.
pub mod binding;
pub mod render;
