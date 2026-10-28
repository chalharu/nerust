//! Frontend view definitions: domain views over the ViewModels.
//!
//! This crate owns the `ViewDefinition` implementations (`HistoryModel`,
//! settings screens, debugger views) and their presentation helpers. It
//! depends on the [`wakui`] framework traits plus the ViewModel crates
//! (`shell`, `viewmodel`, `settings-core`, `runtime`), never on native
//! frontends. Native adapters (`tao`, `gtk`) must not depend on this
//! crate; they drive views through the host.

pub mod application;
pub mod bootstrap;
pub mod commands;
pub mod debugger;
pub mod drain;
pub mod event;
pub mod focus;
pub mod frame;
pub mod history;
pub mod menu;
pub mod osd;
pub mod session;
pub mod settings;
pub mod sync;
pub mod viewport;
pub mod widget_session;
pub mod widgets;
pub mod window;
