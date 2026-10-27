//! Shared high-level host. No native event loop, window, or widget dependency.
//! Native adapters translate callbacks; policy and ordering live here.
//!
//! # Native integration checklist
//!
//! - Own an [`application::ApplicationHost`]; translate native callbacks into
//!   its policy-aware operations rather than duplicating controller state.
//! - Implement [`loop_driver::LoopHost`] and call [`loop_driver::drive_turn`]
//!   at the end of an event burst. Map its deadline/redraw result to the OS pump.
//! - Use [`osd::OsdNotifications`]. Take an owned [`osd::ModalPresentation`],
//!   release controller/`RefCell` borrows, then present native UI. ACK its
//!   [`osd::ModalToken`] through the application host; present chained effects
//!   only after releasing the borrow again. Callbacks may fire synchronously.
//! - Dispatch gestures and poll [`debugger::DebuggerDriver`]; apply owned view
//!   snapshots/effects outside controller borrows. Schedule pending polls.
//! - Use [`settings::SettingsController`] and its apply mailbox; native controls
//!   may bind directly to its view model. Track auxiliary lifetimes with
//!   [`aux::AuxWindows`], reserving before native creation can reenter.
//! - Translate callbacks through [`event::EventSink`] / [`event::EventHandler`]
//!   (both support dyn). Take owned [`event::CommandEffect`] results and release
//!   all controller borrows before native presentation can reenter.
//! - Get owned [`menu::Menubar`] snapshots from
//!   [`application::ApplicationHost::menubar_update`]. Implement
//!   [`commands::CommandVisitor`] declarations using [`commands::CommandId::name`]
//!   and [`commands::CommandDescriptor::bind`], never a native action-name match.
//!   Implement [`menu::MenuVisitor`], including the application root menu.
//!   Submit through [`widgets::MenuPresenter`], release controller borrows, then
//!   drain presentation plans so synchronous reentry always renders newest last.
//! - Retire application liveness, tear down auxiliary/native resources and
//!   flights, then flush with [`application::ApplicationHost::finish_close`].
//!
//! Run `cargo run --locked --offline -p nerust_frontend_host --example native_adapter`
//! for a toolkit-free executable sketch suitable for a future WinUI/AppKit Rust
//! adapter. It checks idle turns, coreless debugger polling and synchronous modal
//! ACK reentry with isolated temporary settings. It is not a native frontend:
//! OS event pumping, toolkit controls, rendering and resource plumbing remain
//! native responsibilities. No total frontend line-count guarantee is implied.

pub mod application;
pub mod aux;
pub mod commands;
pub mod debug_schedule;
pub mod debugger;
pub mod drain;
pub mod event;
pub mod focus;
pub mod frame;
pub mod loop_driver;
pub mod menu;
pub mod osd;
pub mod session;
pub mod settings;
pub mod widgets;
