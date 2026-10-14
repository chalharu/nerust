//! iced-based auxiliary windows, hosted on [`shell::AuxShell`].
//!
//! - [`shell`]: generic tao+iced window scaffold (`UiShell`/`AuxShell`).
//! - [`bridge`]: shared program<->drain bridge (used by `debug`, `ppu`).
//! - [`debug`]: debugger window (program + views).
//! - [`ppu`]: PPU (pattern-table) viewer.
//! - [`settings`]: settings dialog (program + tao handle).

pub(crate) mod bridge;
pub(crate) mod debug;
pub(crate) mod ppu;
pub(crate) mod settings;
pub(crate) mod shell;
