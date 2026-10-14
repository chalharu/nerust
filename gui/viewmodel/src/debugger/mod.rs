//! Debugger view-model: toolkit-independent snapshots in, intents out.
//!
//! Movement state ([`NavState`]) and display data ([`DisplayCache`])
//! are owned separately. One concern per file; everything is
//! re-exported here so the module path is unchanged.

pub mod display;
pub mod format;
pub mod images;
pub mod nav;
pub mod snapshot;
pub mod status;
pub mod transaction;

#[cfg(test)]
mod tests;

pub use display::{DisplayCache, diff_rows};
pub use format::{
    EMPTY_PANELS_TEXT, format_cell, format_disasm_line, format_panels, format_register,
    panels_visible,
};
pub use images::{PPU_HOVER_HINT, rgba_image, scale2x_nearest};
pub use nav::{DISASM_BACK_CAP, NavState, parse_hex_addr};
pub use snapshot::DebugSnapshot;
pub use status::{PendingWrite, StatusMessage, space_key};
pub use transaction::WriteTransaction;
