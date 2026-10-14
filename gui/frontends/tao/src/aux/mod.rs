//! iced-based auxiliary windows, hosted on [`shell::AuxShell`].
//!
//! - [`shell`]: generic tao+iced window scaffold (`UiShell`/`AuxShell`).
//! - [`debug`]: debugger window (bridge + program + views).
//! - [`ppu`]: PPU (pattern-table) viewer.
//! - [`settings`]: settings dialog (program + tao handle).

pub(crate) mod debug;
pub(crate) mod ppu;
pub(crate) mod settings;
pub(crate) mod shell;
