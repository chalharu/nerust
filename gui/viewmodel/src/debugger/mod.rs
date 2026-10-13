//! Debugger view-model: descriptor interpreter plus structured cache.
//!
//! Toolkit-independent: frontends render from snapshots and report
//! intents back. Movement state ([`NavState`]) and display data
//! ([`DisplayCache`]) are owned separately; navigation never reaches
//! into display locks and view code never parses kernel text.
//! Execution and writes stay in the shell session (later phases);
//! this module owns pure interpretation only.
//!
//! One concern per file: `nav` (movement), `display` (DTO + diff),
//! `status` (outcomes), `transaction` (write staging), `snapshot`
//! (drain data), `format` (text), `images` (blit/scale). Everything is
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
