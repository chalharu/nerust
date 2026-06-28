use std::path::PathBuf;

/// CLI options parsed by the root crate and forwarded to the active frontend.
#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    /// Path to a ROM file to load on startup.
    pub rom_path: Option<PathBuf>,
    /// MMC3 IRQ variant override (`"sharp"` or `"nec"`).
    pub mmc3_irq_variant: Option<String>,
}
