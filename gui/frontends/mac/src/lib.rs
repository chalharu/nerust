//! Prototype AppKit-native frontend over the shared frontend host.
//!
//! Same contract as the Win32 twin: [`MacHost`] owns the shared
//! [`ApplicationHost`](nerust_frontend_host::application::ApplicationHost),
//! this crate only translates AppKit-shaped inputs (key codes, menu
//! selectors, activation notifications, display-link ticks) into host
//! events and executes the returned effects through [`MacSurface`].
//!
//! The deliberate difference from the win twin is the platform edge:
//! key-code table, selector strings, and sheet-modal settings flow.
//! Everything behind those — dispatch gates, settings staging, focus
//! protocol, loop ordering — is the same shared code, which is the
//! point: a new native frontend is a mechanical adapter, not a rewrite.

pub mod host;
pub mod keys;
pub mod surface;

pub use host::MacHost;
pub use keys::{mac_key_to_nerust, mac_menu_event};
pub use surface::{HeadlessSurface, MacSurface};
