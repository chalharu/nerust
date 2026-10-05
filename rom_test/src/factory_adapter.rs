//! Single seam to concrete NES construction.
//!
//! Everything the generic validation harness needs — a loaded
//! `Box<dyn ConsoleCore>`, an input writer, and the cartridge-RAM
//! bus-state escape — is built here through the real [`CoreFactory`].
//! No other `rom_test` module names NES-concrete types: `nes_core` is
//! imported only for option/config types and the downcast escape,
//! `nes_factory` / `nes_settings` only for construction.

use std::collections::HashMap;

use nerust_core_traits::{
    ConsoleCore, CoreConfig,
    audio::AudioBackend,
    factory::{
        CoreFactory,
        settings::{FactorySettingsView, Language},
    },
};
use nerust_input_traits::{GuiInput, InputAssignments};
use nerust_nes_core::{
    console_core::NesConsoleCore,
    core_options::{CoreOptions, Mmc3IrqVariant as NesMmc3IrqVariant},
};
use nerust_nes_settings::{NesSettings, NesVideoFilter};

use crate::{error::RomTestError, manifest::Mmc3IrqVariant};

/// A loaded console plus its input writer, ready for headless driving.
pub struct TestSystem {
    pub console: Box<dyn ConsoleCore>,
    pub gui_input: GuiInput,
    /// Display palette from the factory. The filter is pinned to `None`
    /// below, so these bytes equal the validation palette exactly.
    pub palette: Box<[u32; 256]>,
}

/// Build a loaded NES console through `CoreFactory`.
///
/// * Settings come from the factory's own defaults (no GUI involved).
/// * Both player slots get the P1 profile: the suite drives pad2 and
///   the microphone, which the factory default (P2 unassigned) lacks.
/// * Audio goes to a null backend; the harness collects production
///   audio through `render_frame`'s caller buffer instead.
/// * Test options (`mmc3_irq_variant`, …) travel in
///   `CoreConfig::core_options`, which the console downcasts.
pub fn open_nes_system(
    factory: &dyn CoreFactory,
    case_id: &str,
    rom_bytes: &[u8],
    mmc3_irq_variant: Option<Mmc3IrqVariant>,
    audio_sample_rate: u32,
) -> Result<TestSystem, RomTestError> {
    let construction = |message: String| RomTestError::CoreConstruction {
        case_id: case_id.to_string(),
        message,
    };

    let mut system_config = factory
        .as_system_defaults()
        .and_then(|defaults| defaults.default_system_settings())
        .ok_or_else(|| construction("factory provides no default settings".to_string()))?;
    // Pin the video filter to `None` for determinism: screenshots and
    // palette bytes must not depend on GUI defaults. Loud failure keeps
    // a future settings change from silently altering hashes.
    let nes_settings = system_config
        .downcast_mut::<NesSettings>()
        .ok_or_else(|| construction("cannot pin NES video filter".to_string()))?;
    nes_settings.video.filter = NesVideoFilter::None;
    let view = FactorySettingsView {
        language: Language::English,
        system_config: Some(system_config),
    };

    let base = factory.input_system_factory().default_assignments();
    let assignments = duplicate_p1_to_p2(&base)
        .ok_or_else(|| construction("factory default assigns no P1 profile".to_string()))?;

    let speaker: Box<dyn AudioBackend> = Box::new(NullAudioBackend);
    let mut parts = factory
        .create_core_and_adapter_with_assignments(&view, speaker, &assignments)
        .map_err(|error| construction(format!("create_core: {error:?}")))?;

    let config = CoreConfig {
        region: None,
        bios_paths: HashMap::new(),
        controllers: HashMap::new(),
        core_options: Some(Box::new(core_options_for(mmc3_irq_variant))),
        // The console stamps this into its resampler. It must match the
        // mixer's rate: the old direct path sampled `audio.sample_rate()`.
        audio_sample_rate: Some(audio_sample_rate),
    };
    parts
        .core
        .load(rom_bytes, &config)
        .map_err(|error| construction(format!("load: {error:?}")))?;

    // Factory-owned display palette. The filter is pinned to `None`
    // above, so these are the validation palette bytes exactly.
    let palette: Box<[u32; 256]> = parts
        .palette
        .into_vec()
        .try_into()
        .map_err(|_| construction("factory palette is not 256 entries".to_string()))?;

    Ok(TestSystem {
        console: parts.core,
        gui_input: parts.gui_input,
        palette,
    })
}

/// Build concrete NES options from the manifest-schema variant.
/// The single place that maps test schema types to core types.
pub(crate) fn core_options_for(mmc3_irq_variant: Option<Mmc3IrqVariant>) -> CoreOptions {
    CoreOptions {
        mmc3_irq_variant: mmc3_irq_variant.map(|variant| match variant {
            Mmc3IrqVariant::Sharp => NesMmc3IrqVariant::Sharp,
            Mmc3IrqVariant::Nec => NesMmc3IrqVariant::Nec,
        }),
    }
}

/// Clone the P1 profile onto P2. The suite drives both pads (88 pad2
/// events) plus the microphone, so the factory default of an unassigned
/// P2 is insufficient here.
fn duplicate_p1_to_p2(base: &InputAssignments) -> Option<InputAssignments> {
    let p1 = base.slots.first()?.1.clone()?;
    let mut slots = base.slots.clone();
    if slots.len() > 1 {
        slots[1].1 = Some(p1);
    }
    Some(InputAssignments { slots })
}

impl TestSystem {
    /// Publish absolute pad state for the next frame.
    ///
    /// Bit layouts match `NesInputBuffer`: fields 0-7 pad1
    /// (A B Select Start Up Down Left Right), 8-15 pad2, 16 mic.
    pub fn sync_input(&mut self, pad1: u8, pad2: u8, mic: bool) {
        if let Some(buffer) = self
            .gui_input
            .state
            .downcast_mut::<nerust_nes_core::input_types::NesInputBuffer>()
        {
            buffer.0 = [pad1, pad2, u8::from(mic)];
        }
        self.gui_input.publish();
    }
}

/// Cartridge-RAM peek preserving bus state (mapped vs open bus).
///
/// Reached through a downcast escape confined to this module: the
/// generic debugger read path carries values only, and cartridge
/// assertions compare bus state too. Generic harness code never names
/// NES types; the downcast lives here.
pub fn peek_cartridge_ram(console: &dyn ConsoleCore, address: usize) -> Option<(u8, bool)> {
    let nes = console.downcast_ref::<NesConsoleCore>()?;
    nes.peek_cartridge_ram(address)
        .map(|read| (read.data, read.mask != 0xFF))
}

/// Silent audio backend for headless runs. Production audio is
/// collected through `render_frame`'s caller-provided buffer.
struct NullAudioBackend;

impl AudioBackend for NullAudioBackend {
    fn start(&mut self) {}
    fn pause(&mut self) {}
    fn push(&mut self, _sample: nerust_core_traits::audio::StereoSample) {}
    fn sample_rate(&self) -> u32 {
        48_000
    }
}
