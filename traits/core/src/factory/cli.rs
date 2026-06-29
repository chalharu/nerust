use clap::{ArgMatches, Command};

/// Provides system-specific CLI argument definitions and parsing.
///
/// Each system factory (NES, SNES, etc.) implements this trait to
/// define its own command-line options (e.g. `--mmc3-irq-variant`).
/// The root crate collects all args and delegates parsing back.
pub trait CliProvider: Send + Sync {
    /// Extend a clap `Command` with system-specific arguments.
    fn extend_command(&self, cmd: Command) -> Command;

    /// Parse core options from matched CLI arguments.
    ///
    /// Returns opaque bytes interpreted by the factory's
    /// `resolve_load_request` / builder.
    fn parse_core_options(&self, matches: &ArgMatches) -> Vec<u8>;
}
