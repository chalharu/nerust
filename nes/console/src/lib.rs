use std::sync::Arc;

use nerust_contract_core::audio::AudioBackend;
use nerust_contract_core::device::Device;
use nerust_contract_core::{
    ConsoleCore, CoreCapabilities, CoreConfig, CoreError, GpuCommand, GpuCommandList, PixelFormat,
    VideoSignalKind,
};
use nerust_input_nes_runtime::nes_input_cell::{NesInputCell, SharedNesInputCell};
use nerust_nes_core::Core;
use nerust_nes_core::cartridge_rom::CartridgeData;
use nerust_nes_device::nes_pad::NesPadDevice;
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_sound_traits::MixerInput;
use nerust_soundfilter::Filter;
use nerust_soundfilter::NesFilter;
use nerust_soundfilter::resampler::Resampler;
use nerust_soundfilter::resampler::SimpleDownSampler;

const CLOCK_RATE: u32 = 1_789_773;
const OVERSAMPLE_FACTOR: u32 = 4;

fn oversampled_rate(device_rate: u32) -> u32 {
    device_rate
        .saturating_mul(OVERSAMPLE_FACTOR)
        .min(CLOCK_RATE)
        .max(device_rate)
}

/// Temporary adapter: converts `Box<dyn AudioBackend>` to `MixerInput`.
///
/// Phase 7 で `run_frame` が `AudioBackend` 直受けになった時点で不要になる。
/// それまでは、`MixerBridge` 相当のオーバーサンプリング + NesFilter + ダウンサンプリングを提供する。
struct AudioBackendAdapter {
    backend: Box<dyn AudioBackend>,
    source_rate: u32,
    gain: f32,
    filter: NesFilter,
    resampler: SimpleDownSampler,
}

impl AudioBackendAdapter {
    fn new(backend: Box<dyn AudioBackend>, gain: f32) -> Self {
        let device_rate = backend.sample_rate();
        let source_rate = oversampled_rate(device_rate);
        Self {
            backend,
            source_rate,
            gain: gain.clamp(0.0, 1.0),
            filter: NesFilter::new(device_rate as f32),
            resampler: SimpleDownSampler::new(f64::from(source_rate), f64::from(device_rate)),
        }
    }
}

impl MixerInput for AudioBackendAdapter {
    fn push(&mut self, data: f32) {
        if let Some(resampled) = self.resampler.step(data) {
            let sample = self.filter.step((resampled * 2.0 - 1.0) * self.gain);
            self.backend.push(sample);
        }
    }

    fn sample_rate(&self) -> u32 {
        self.source_rate
    }
}

/// NES ConsoleCore 実装（初版）
///
/// 内部で既存の `Core::run_frame()` を呼び出し、`ScreenBuffer` に描画し、
/// `frame_slot` に書き出す。音声は `AudioBackendAdapter` を介して出力する。
pub struct NesConsoleCore {
    core: Core,
    screen: ScreenBuffer,
    controller: NesPadDevice<SharedNesInputCell>,
    adapter: AudioBackendAdapter,
}

impl NesConsoleCore {
    pub fn new(
        rom: CartridgeData,
        backend: Box<dyn AudioBackend>,
        gain: f32,
    ) -> Result<Self, CoreError> {
        let core = Core::new(rom).map_err(|e| CoreError::Core(e.to_string()))?;
        let screen = ScreenBuffer::new_nes_gpu_default();
        let cell = Arc::new(NesInputCell::new());
        let controller = NesPadDevice::new(SharedNesInputCell(cell));
        Ok(Self {
            core,
            screen,
            controller,
            adapter: AudioBackendAdapter::new(backend, gain),
        })
    }
}

impl ConsoleCore for NesConsoleCore {
    fn capabilities(&self) -> CoreCapabilities {
        CoreCapabilities {
            output_formats: vec![PixelFormat::Rgba],
            video_signal: VideoSignalKind::Ntsc,
        }
    }

    fn render_frame(&mut self, frame_slot: &mut [u8]) -> Result<GpuCommandList, CoreError> {
        self.core
            .run_frame(&mut self.screen, &mut self.controller, &mut self.adapter);
        self.screen.write_frame_into(frame_slot);
        Ok(GpuCommandList {
            commands: vec![GpuCommand::Blit { slot: 0 }],
        })
    }

    fn frame_slot_size(&self) -> usize {
        self.screen.frame_len()
    }

    fn audio_samples(&self, _out: &mut dyn AudioBackend) {}

    fn attach_device(&mut self, _port: usize, _device: Box<dyn Device>) {}

    fn detach_device(&mut self, _port: usize) {}

    fn load(&mut self, _rom: &[u8], _config: &CoreConfig) -> Result<(), CoreError> {
        Err(CoreError::Core("hot-load not supported".into()))
    }

    fn unload(&mut self) {}

    fn reset(&mut self) {
        self.core.reset();
    }

    fn paused(&self) -> bool {
        false
    }

    fn set_paused(&mut self, _paused: bool) {}

    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        self.core
            .export_machine_state()
            .map_err(|e| CoreError::Core(e.to_string()))
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError> {
        self.core
            .import_machine_state(data)
            .map_err(|e| CoreError::Core(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_contract_core::audio::NullAudio;
    use nerust_contract_core::mirror::MirrorMode;
    use nerust_contract_core::rom::RomFormat;
    use nerust_nes_core::cartridge_data_parts::CartridgeDataParts;
    use nerust_nes_core::cartridge_rom::CartridgeData;

    fn nrom_test_data() -> CartridgeData {
        CartridgeData::new(CartridgeDataParts {
            format: RomFormat::INes,
            prog_rom: vec![0; 0x8000],
            char_rom: vec![0; 0x2000],
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 0,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid")
    }

    #[test]
    fn nes_console_core_constructs_with_null_audio() {
        let backend = Box::new(NullAudio);
        let mut console = NesConsoleCore::new(nrom_test_data(), backend, 1.0)
            .expect("NesConsoleCore should construct");
        let slot_size = console.frame_slot_size();
        assert!(slot_size > 0);
        let mut slot = vec![0u8; slot_size];
        let cmds = console
            .render_frame(&mut slot)
            .expect("render_frame should succeed");
        assert_eq!(cmds.commands.len(), 1);
        assert!(matches!(cmds.commands[0], GpuCommand::Blit { slot: 0 }));
    }

    #[test]
    fn nes_console_core_produces_deterministic_frames() {
        let backend = Box::new(NullAudio);
        let mut console = NesConsoleCore::new(nrom_test_data(), backend, 1.0)
            .expect("NesConsoleCore should construct");
        let slot_size = console.frame_slot_size();
        let mut slot1 = vec![0u8; slot_size];
        let mut slot2 = vec![0u8; slot_size];
        console.render_frame(&mut slot1).expect("frame 0");
        console.render_frame(&mut slot2).expect("frame 1");
        assert_eq!(slot1, slot2, "frames should be deterministic");
    }

    #[test]
    fn nes_console_core_save_state_round_trips() {
        let backend = Box::new(NullAudio);
        let mut console = NesConsoleCore::new(nrom_test_data(), backend, 1.0)
            .expect("NesConsoleCore should construct");
        let slot_size = console.frame_slot_size();
        let mut slot = vec![0u8; slot_size];
        console.render_frame(&mut slot).expect("render before save");
        let state = console.save_state().expect("save state");
        let mut restored = NesConsoleCore::new(nrom_test_data(), Box::new(NullAudio), 1.0)
            .expect("restored NesConsoleCore should construct");
        restored.load_state(&state).expect("load state");
        let mut restored_slot = vec![0u8; slot_size];
        restored
            .render_frame(&mut restored_slot)
            .expect("render after restore");
        assert_eq!(
            slot, restored_slot,
            "state restore should produce identical frame"
        );
    }
}
