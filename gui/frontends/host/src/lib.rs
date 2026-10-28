//! Framework integration host: boundary vocabulary, ports, and the app
//! trait. No native event loop, window, or widget dependency, and no
//! intra-repo dependencies beyond [`wakui`].
//!
//! Native adapters own resources and translate callbacks; policy and
//! ordering live view-side behind [`app::FrontendApp`], injected as a
//! trait object by the composition root (see [`app::Boot`]).
//!
//! # Native integration checklist
//!
//! - Receive a [`app::Boot`] and drive `app` through [`app::FrontendApp`]
//!   ports rather than duplicating controller state.
//! - Implement [`wakui::loop_driver::LoopHost`] and call
//!   [`wakui::loop_driver::drive_turn`] at the end of an event burst. Map
//!   its deadline/redraw result to the OS pump.
//! - Pump OSD through [`app::FrontendApp::pump_osd`], present owned
//!   [`osd::ModalPresentation`] through `dyn` [`osd::ModalPresenter`].
//!   ACK with `acknowledge_osd_modal`; release borrows again before
//!   presenting chained effects. Callbacks may reenter.
//! - Dispatch gestures through [`event::EventHandler`] (implemented by the
//!   app) and take owned [`event::CommandEffect`] results.
//! - Render owned [`menu::Menubar`] snapshots from `menubar_update`.
//!   Implement [`menu::MenuVisitor`]; submit through
//!   [`widgets::MenuPresenter`], release controller borrows, then drain
//!   presentation plans so synchronous reentry always renders newest last.
//! - Prepare Settings and History with `prepare_widget`; render owned
//!   [`wakui::control::ControlScreen`] snapshots; send opaque
//!   [`wakui::control::ControlEvent`] callbacks to `widget_dispatch`.
//!   Native registries retain [`widget_session::WindowMailbox`], never
//!   settings drafts or domain bindings. Reserve [`aux::AuxWindows`]
//!   before native creation.

pub mod app;
pub mod aux;
pub mod bootstrap;
pub mod commands;
pub mod debug_schedule;
pub mod debugger;
pub mod event;
pub mod focus;
pub mod menu;
pub mod osd;
pub mod session;
pub mod sync;
pub mod viewport;
pub mod widget_session;
pub mod widgets;
pub mod window;
