//! iced-based auxiliary windows on [`shell::AuxShell`].
//! [`bridge`] state plus [`debug`], [`ppu`], [`settings`], [`focus`] windows.

pub(crate) mod bridge;
pub(crate) mod debug;
pub(crate) mod focus;
pub(crate) mod native;
pub(crate) mod osd;
pub(crate) mod ppu;
pub(crate) mod registry;
pub(crate) mod settings;
pub(crate) mod shell;
pub(crate) mod tao_map;
