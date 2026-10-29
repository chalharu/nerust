//! Turn flow: the [`wakui_core::flow`] driver owned end to end. [`runtime`]
//! owns the tao event loop, [`window_events`] routes per-window events,
//! [`widget_drain`] pumps polled widgets, and [`commands`] dispatches menu,
//! keyboard, and file input into host actions.
pub mod commands;
pub mod runtime;
pub mod widget_drain;
pub mod window_events;
