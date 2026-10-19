//! iced-based auxiliary windows on [`shell::AuxShell`]: [`shell`]
//! scaffold, [`bridge`] shared state, [`debug`], [`ppu`], [`settings`],
//! plus the [`focus`] agreement every aux window honors.

pub(crate) mod bridge;
pub(crate) mod debug;
pub(crate) mod focus;
pub(crate) mod osd;
pub(crate) mod ppu;
pub(crate) mod settings;
pub(crate) mod shell;
