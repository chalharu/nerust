//! Shared high-level host. No native event loop, window, or widget dependency.
//! Native adapters translate callbacks; policy and ordering live here.
//!
//! # Native integration checklist
//!
//! - Own an [`application::ApplicationHost`]; translate native callbacks into
//!   its policy-aware operations rather than duplicating controller state.
//! - Implement [`loop_driver::LoopHost`] and call [`loop_driver::drive_turn`]
//!   at the end of an event burst. Map its deadline/redraw result to the OS pump.
//! - Let [`application::ApplicationHost`] own OSD state. Call
//!   [`application::ApplicationHost::pump_osd`] and
//!   [`application::ApplicationHost::prepare_osd`] under the host borrow, then
//!   release it. Deliver owned [`osd::OsdUpdate`] through `dyn` [`osd::OsdVisitor`]
//!   and owned [`osd::ModalPresentation`] through `dyn` [`osd::ModalPresenter`].
//!   ACK with [`application::ApplicationHost::acknowledge_osd_modal`]; release
//!   borrows again before presenting chained effects. Callbacks may reenter.
//!   Invalidate OSD output when the destination is replaced or delivery abandoned.
//! - Use [`history::HistoryController`] to gate session history updates, or cache
//!   an owned [`history::HistoryModel`] built on sync/filter messages. Release
//!   controller/host borrows before visiting through `dyn` [`history::HistoryVisitor`].
//!   The model owns captions and rendered rows; only native controls stay local.
//! - Dispatch gestures and poll [`debugger::DebuggerDriver`]; apply owned view
//!   snapshots/effects outside controller borrows. Schedule pending polls.
//! - Prepare Settings and History with `ApplicationHost::prepare_widget_window`.
//!   Render owned [`control::ControlScreen`] snapshots using `visit_controls`;
//!   send opaque [`control::ControlEvent`] callbacks to [`widget_session::WidgetSession`].
//!   Native registries retain [`widget_session::WindowMailbox`], never settings
//!   drafts or domain bindings. Reserve [`aux::AuxWindows`] before native creation.
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
pub mod binding;
pub mod bootstrap;
pub mod commands;
pub mod control;
pub mod debug_schedule;
pub mod debugger;
pub mod drain;
pub mod event;
pub mod focus;
pub mod frame;
pub mod history;
pub mod loop_driver;
pub mod mapper;
pub mod menu;
pub mod osd;
pub mod session;
pub mod settings;
pub mod viewport;
pub mod widget_session;
pub mod widgets;
pub mod window;
