use std::fmt::{Debug, Display};

/// System-agnostic memory space identifier.
///
/// Each console system defines its own enum implementing this trait
/// (typically via `strum` derive macros).
///
/// Intended for use as a trait object (`&dyn MemorySpace`).
pub trait MemorySpace: Debug + Display + Send + Sync + 'static {
    /// Unique identifier for comparison (e.g., "cpu", "ppu", "oam").
    fn id(&self) -> &'static str;

    /// Human-readable name for UI display (e.g., "CPU Bus", "Video Memory").
    fn name(&self) -> &'static str;

    /// Address bus width in bits (e.g., 16 for a 64 KiB space).
    fn address_bits(&self) -> u8;
}
