//! Native chrome: windows, menus, dialogs, and auxiliary-window routing.
//! Snapshots flow view-to-native; native effects flow back through ports.
pub mod aux;
pub mod dialog;
pub mod mailbox;
pub mod menu;
pub mod menu_presenter;
pub mod viewport;
pub mod window;
