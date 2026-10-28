//! iced-based auxiliary windows on [`shell::AuxShell`].
//! Native grouped widgets share the outer Window traversal.

pub(crate) mod bridge;
pub(crate) mod debug;
pub(crate) mod focus;
pub(crate) mod native;
pub(crate) mod ppu;
pub(crate) mod registry;
pub(crate) mod shell;
pub(crate) mod widgets;
pub(crate) mod window;
