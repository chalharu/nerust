//! Tao-side input chain: toolkit events to framework [`wakui_core::input`] vocab.
//! [`vocab`] is the toolkit-independent vocabulary, [`tao_map`] lifts tao
//! events into it, [`convert`] feeds it to iced, and [`keyboard`] maps
//! physical keys both ways.
pub mod convert;
pub mod keyboard;
pub mod tao_map;
pub mod vocab;
