use super::{
    RuntimeHostServices, SystemDefinition, SystemDescriptor, SystemInputAdapter, SystemRuntime,
    SystemRuntimeSnapshot, SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel,
};
use crate::load::{MediaObject, ResolvedLoadRequest, SystemLoadOptions};
use nerust_console::ConsoleMetrics;
use nerust_console::state::RuntimeStateExport;
use nerust_console::video::{VideoFrameHandle, VideoRenderProfile};
use nerust_contract_persistence::CanonicalMediaIdentity;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_input_schema::{
    AttachmentId, AttachmentSlotDescriptor, ControlDescriptor, DeviceDescriptor, DeviceKindId,
    DigitalControlDescriptor, DigitalControlId, DigitalInputEvent, InputTopologyDescriptor,
    PortDescriptor, PortId, SystemId,
};
use nerust_screen_logical::LogicalSize;
use nerust_screen_physical::PhysicalSize;
use nerust_snes_core::{Core, CpuState};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const SCREEN_WIDTH: usize = 256;
const SCREEN_HEIGHT: usize = 224;
const FRAME_STRIDE_BYTES: usize = SCREEN_WIDTH * 4;
const CPU_CYCLES_PER_FRAME: u64 = 60_000;
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

const SNES_PORT_ONE: PortId = PortId::new("snes.port.controller1");
const SNES_ATTACHMENT_CONTROLLER_ONE: AttachmentId =
    AttachmentId::new("snes.attachment.controller1");
const SNES_STANDARD_PAD: DeviceKindId = DeviceKindId::new("snes.device.standard_pad");
const SNES_CONTROL_B: DigitalControlId = DigitalControlId::new("snes.control.b");
const SNES_CONTROL_Y: DigitalControlId = DigitalControlId::new("snes.control.y");
const SNES_CONTROL_SELECT: DigitalControlId = DigitalControlId::new("snes.control.select");
const SNES_CONTROL_START: DigitalControlId = DigitalControlId::new("snes.control.start");
const SNES_CONTROL_UP: DigitalControlId = DigitalControlId::new("snes.control.up");
const SNES_CONTROL_DOWN: DigitalControlId = DigitalControlId::new("snes.control.down");
const SNES_CONTROL_LEFT: DigitalControlId = DigitalControlId::new("snes.control.left");
const SNES_CONTROL_RIGHT: DigitalControlId = DigitalControlId::new("snes.control.right");
const SNES_CONTROL_A: DigitalControlId = DigitalControlId::new("snes.control.a");
const SNES_CONTROL_X: DigitalControlId = DigitalControlId::new("snes.control.x");
const SNES_CONTROL_L: DigitalControlId = DigitalControlId::new("snes.control.l");
const SNES_CONTROL_R: DigitalControlId = DigitalControlId::new("snes.control.r");

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct SnesSystemDefinition;

#[derive(Debug, Default)]
struct SnesInputAdapter {
    buttons: u16,
}

pub(super) fn media_looks_like_snes(media: &MediaObject) -> bool {
    matches!(media.extension.as_deref(), Some("sfc" | "smc"))
        || nerust_snes_core::Cartridge::from_bytes(media.bytes.as_ref()).is_ok()
}

impl SystemDefinition for SnesSystemDefinition {
    fn descriptor(&self) -> SystemDescriptor {
        SystemDescriptor {
            system_id: SystemId::Snes,
            input_topology: snes_input_topology_descriptor(),
        }
    }

    fn probe_media(&self, media: &MediaObject) -> bool {
        media_looks_like_snes(media)
    }

    fn default_load_options(&self) -> SystemLoadOptions {
        SystemLoadOptions::default()
    }

    fn resolve_load_request(
        &self,
        _settings: &SettingsSnapshot,
        options: SystemLoadOptions,
    ) -> Result<ResolvedLoadRequest, String> {
        Ok(ResolvedLoadRequest {
            system_id: SystemId::Snes,
            options,
            core_options: options.into_core_options(),
        })
    }

    fn settings_page(&self, _settings: &SettingsSnapshot) -> SystemSettingsPageModel {
        SystemSettingsPageModel {
            fields: Arc::from([]),
        }
    }

    fn apply_settings_choice(
        &self,
        _settings: &mut SettingsSnapshot,
        field: &SystemSettingsFieldId,
        _choice: &SystemSettingsChoiceId,
    ) -> Result<(), String> {
        Err(format!(
            "unsupported SNES system settings field: {}",
            field.as_str()
        ))
    }

    fn create_input_adapter(&self, _settings: &SettingsSnapshot) -> Box<dyn SystemInputAdapter> {
        Box::<SnesInputAdapter>::default()
    }

    fn create_runtime(
        &self,
        _host: &RuntimeHostServices,
        _settings: &SettingsSnapshot,
    ) -> Result<Box<dyn SystemRuntime>, String> {
        Ok(Box::new(SnesRuntime::new()))
    }
}

impl SystemInputAdapter for SnesInputAdapter {
    fn digital_event_from_persisted(
        &self,
        attachment: &str,
        control: &str,
        pressed: bool,
    ) -> Option<DigitalInputEvent> {
        if attachment != SNES_ATTACHMENT_CONTROLLER_ONE.as_str() {
            return None;
        }
        let control = snes_control_from_persisted(control)?;
        Some(if pressed {
            DigitalInputEvent::pressed(SNES_ATTACHMENT_CONTROLLER_ONE, control)
        } else {
            DigitalInputEvent::released(SNES_ATTACHMENT_CONTROLLER_ONE, control)
        })
    }

    fn apply_event(&mut self, event: DigitalInputEvent) {
        if event.attachment != SNES_ATTACHMENT_CONTROLLER_ONE {
            return;
        }
        if let Some(mask) = button_mask(event.control) {
            if event.is_pressed() {
                self.buttons |= mask;
            } else {
                self.buttons &= !mask;
            }
        }
    }

    fn clear(&mut self) {
        self.buttons = 0;
    }

    fn sync_from_runtime_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.buttons = decode_buttons(bytes)?;
        Ok(())
    }

    fn runtime_state_bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.buttons.to_le_bytes().to_vec())
    }
}

pub(super) struct SnesRuntime {
    commands: Sender<SnesCommand>,
    stop: Sender<()>,
    thread: Option<JoinHandle<()>>,
    shared: Arc<SnesRuntimeShared>,
}

struct SnesRuntimeShared {
    metrics: RwLock<ConsoleMetrics>,
    frame: RwLock<Arc<[u8]>>,
    input_buttons: RwLock<u16>,
}

enum SnesCommand {
    Load {
        bytes: Vec<u8>,
        msu1_data: Option<Vec<u8>>,
        msu1_audio_tracks: Vec<u16>,
        reply: Sender<Result<(), String>>,
    },
    Unload {
        reply: Sender<Result<bool, String>>,
    },
    Reset {
        reply: Sender<Result<(), String>>,
    },
    Pause,
    Resume,
    ApplyInput {
        buttons: u16,
    },
    ExportMapperSave {
        reply: Sender<Result<Option<Vec<u8>>, String>>,
    },
    ImportMapperSave {
        bytes: Vec<u8>,
        reply: Sender<Result<(), String>>,
    },
}

impl SnesRuntime {
    fn new() -> Self {
        let (commands, command_receiver) = mpsc::channel();
        let (stop, stop_receiver) = mpsc::channel();
        let shared = Arc::new(SnesRuntimeShared::new());
        let worker_shared = shared.clone();
        let thread =
            thread::spawn(move || run_worker(command_receiver, stop_receiver, worker_shared));
        Self {
            commands,
            stop,
            thread: Some(thread),
            shared,
        }
    }

    fn request_unit(
        &self,
        command: impl FnOnce(Sender<Result<(), String>>) -> SnesCommand,
    ) -> Result<(), String> {
        let (reply, receiver) = mpsc::channel();
        self.commands
            .send(command(reply))
            .map_err(|_| "SNES runtime worker is unavailable".to_string())?;
        receiver
            .recv()
            .map_err(|_| "SNES runtime worker is unavailable".to_string())?
    }
}

impl SystemRuntime for SnesRuntime {
    fn snapshot(&self) -> SystemRuntimeSnapshot {
        SystemRuntimeSnapshot {
            metrics: self.shared.metrics(),
            video_frame: Some(self.shared.video_frame()),
            video_profile: Some(snes_video_profile()),
        }
    }

    fn load(&mut self, media: &MediaObject, _request: &ResolvedLoadRequest) -> Result<(), String> {
        let msu1_sidecars = load_msu1_sidecars(media.path.as_deref())?;
        self.request_unit(|reply| SnesCommand::Load {
            bytes: media.bytes.as_ref().to_vec(),
            msu1_data: msu1_sidecars.data,
            msu1_audio_tracks: msu1_sidecars.audio_tracks,
            reply,
        })
    }

    fn unload(&mut self) -> Result<bool, String> {
        let (reply, receiver) = mpsc::channel();
        self.commands
            .send(SnesCommand::Unload { reply })
            .map_err(|_| "SNES runtime worker is unavailable".to_string())?;
        receiver
            .recv()
            .map_err(|_| "SNES runtime worker is unavailable".to_string())?
    }

    fn reset(&self) -> Result<(), String> {
        self.request_unit(|reply| SnesCommand::Reset { reply })
    }

    fn pause(&mut self) {
        if self.commands.send(SnesCommand::Pause).is_err() {
            log::warn!("SNES runtime pause send failed");
        }
    }

    fn resume(&mut self) {
        if self.commands.send(SnesCommand::Resume).is_err() {
            log::warn!("SNES runtime resume send failed");
        }
    }

    fn apply_input_state(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        let buttons = decode_buttons(&bytes)?;
        self.shared.set_input_buttons(buttons);
        self.commands
            .send(SnesCommand::ApplyInput { buttons })
            .map_err(|_| "SNES runtime worker is unavailable".to_string())
    }

    fn current_input_state(&self) -> Result<Vec<u8>, String> {
        Ok(self.shared.input_buttons().to_le_bytes().to_vec())
    }

    fn export_state(&self) -> Result<RuntimeStateExport, String> {
        Err("SNES runtime state export is not supported yet".into())
    }

    fn import_state(&mut self, _state_blob: &[u8]) -> Result<(), String> {
        Err("SNES runtime state import is not supported yet".into())
    }

    fn export_mapper_save(&self) -> Result<Option<Vec<u8>>, String> {
        let (reply, receiver) = mpsc::channel();
        self.commands
            .send(SnesCommand::ExportMapperSave { reply })
            .map_err(|_| "SNES runtime worker is unavailable".to_string())?;
        receiver
            .recv()
            .map_err(|_| "SNES runtime worker is unavailable".to_string())?
    }

    fn import_mapper_save(&self, bytes: Vec<u8>) -> Result<(), String> {
        self.request_unit(|reply| SnesCommand::ImportMapperSave { bytes, reply })
    }

    fn canonical_media_identity(&self) -> Option<CanonicalMediaIdentity> {
        None
    }
}

impl Drop for SnesRuntime {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        let _ = self.thread.take().map(JoinHandle::join);
    }
}

impl SnesRuntimeShared {
    fn new() -> Self {
        Self {
            metrics: RwLock::new(ConsoleMetrics {
                paused: true,
                ..ConsoleMetrics::default()
            }),
            frame: RwLock::new(Arc::from(opaque_black_frame())),
            input_buttons: RwLock::new(0),
        }
    }

    fn metrics(&self) -> ConsoleMetrics {
        *self.metrics.read().unwrap_or_else(|err| err.into_inner())
    }

    fn publish_metrics(&self, metrics: ConsoleMetrics) {
        *self.metrics.write().unwrap_or_else(|err| err.into_inner()) = metrics;
    }

    fn publish_frame(&self, frame: Vec<u8>) {
        *self.frame.write().unwrap_or_else(|err| err.into_inner()) = Arc::from(frame);
    }

    fn video_frame(&self) -> VideoFrameHandle {
        let frame = self.frame.read().unwrap_or_else(|err| err.into_inner());
        VideoFrameHandle::new(
            SCREEN_WIDTH as u32,
            SCREEN_HEIGHT as u32,
            FRAME_STRIDE_BYTES,
            frame.clone(),
        )
    }

    fn input_buttons(&self) -> u16 {
        *self
            .input_buttons
            .read()
            .unwrap_or_else(|err| err.into_inner())
    }

    fn set_input_buttons(&self, buttons: u16) {
        *self
            .input_buttons
            .write()
            .unwrap_or_else(|err| err.into_inner()) = buttons;
    }
}

fn run_worker(commands: Receiver<SnesCommand>, stop: Receiver<()>, shared: Arc<SnesRuntimeShared>) {
    let mut core: Option<Core> = None;
    let mut loaded = false;
    let mut paused = true;
    let mut frame_counter = 0_u64;
    let mut input_buttons = 0_u16;
    let mut last_frame_at = Instant::now();
    publish_metrics(&shared, frame_counter, loaded, paused, 0.0);

    while stop.try_recv().is_err() {
        while let Ok(command) = commands.try_recv() {
            handle_command(
                command,
                &mut core,
                &mut loaded,
                &mut paused,
                &mut frame_counter,
                &mut input_buttons,
                &shared,
            );
        }

        if !loaded || paused {
            thread::sleep(Duration::from_millis(2));
            continue;
        }

        let frame_started = Instant::now();
        if let Some(core) = core.as_mut() {
            match step_snes_frame(core) {
                Ok(()) => {
                    shared.publish_frame(render_backdrop_frame(core));
                    frame_counter = frame_counter.wrapping_add(1);
                    let elapsed = last_frame_at.elapsed().as_secs_f32();
                    last_frame_at = Instant::now();
                    let fps = if elapsed > 0.0 { 1.0 / elapsed } else { 0.0 };
                    publish_metrics(&shared, frame_counter, loaded, paused, fps);
                }
                Err(error) => {
                    log::warn!("SNES runtime paused after core error: {error}");
                    paused = true;
                    publish_metrics(&shared, frame_counter, loaded, paused, 0.0);
                }
            }
        }

        let elapsed = frame_started.elapsed();
        if elapsed < FRAME_INTERVAL {
            thread::sleep(FRAME_INTERVAL - elapsed);
        }
    }
}

fn handle_command(
    command: SnesCommand,
    core: &mut Option<Core>,
    loaded: &mut bool,
    paused: &mut bool,
    frame_counter: &mut u64,
    input_buttons: &mut u16,
    shared: &SnesRuntimeShared,
) {
    match command {
        SnesCommand::Load {
            bytes,
            msu1_data,
            msu1_audio_tracks,
            reply,
        } => {
            let core_result = Core::from_rom_bytes_with_msu1_sidecars(
                &bytes,
                msu1_data.as_deref(),
                &msu1_audio_tracks,
            );
            let result = match core_result {
                Ok(new_core) => {
                    *core = Some(new_core);
                    if let Some(core) = core.as_mut() {
                        core.set_standard_controller_buttons(0, *input_buttons);
                    }
                    *loaded = true;
                    *paused = true;
                    *frame_counter = 0;
                    if let Some(core) = core.as_ref() {
                        shared.publish_frame(render_backdrop_frame(core));
                    }
                    publish_metrics(shared, *frame_counter, *loaded, *paused, 0.0);
                    Ok(())
                }
                Err(error) => Err(error.to_string()),
            };
            send_reply(reply, result);
        }
        SnesCommand::Unload { reply } => {
            let was_loaded = core.take().is_some();
            *loaded = false;
            *paused = false;
            *frame_counter = 0;
            shared.publish_frame(opaque_black_frame());
            publish_metrics(shared, *frame_counter, *loaded, *paused, 0.0);
            send_reply(reply, Ok(was_loaded));
        }
        SnesCommand::Reset { reply } => {
            let result = match core.as_mut() {
                Some(core) => {
                    core.reset_cpu();
                    core.set_standard_controller_buttons(0, *input_buttons);
                    *frame_counter = 0;
                    shared.publish_frame(render_backdrop_frame(core));
                    publish_metrics(shared, *frame_counter, *loaded, *paused, 0.0);
                    Ok(())
                }
                None => Err("no SNES ROM loaded".to_string()),
            };
            send_reply(reply, result);
        }
        SnesCommand::Pause => {
            *paused = true;
            publish_metrics(shared, *frame_counter, *loaded, *paused, 0.0);
        }
        SnesCommand::Resume => {
            if *loaded {
                *paused = false;
                publish_metrics(shared, *frame_counter, *loaded, *paused, 0.0);
            }
        }
        SnesCommand::ApplyInput { buttons } => {
            *input_buttons = buttons;
            if let Some(core) = core.as_mut() {
                core.set_standard_controller_buttons(0, buttons);
            }
        }
        SnesCommand::ExportMapperSave { reply } => {
            let result = core
                .as_ref()
                .map(|core| core.export_save_ram())
                .ok_or_else(|| "no SNES ROM loaded".to_string());
            send_reply(reply, result);
        }
        SnesCommand::ImportMapperSave { bytes, reply } => {
            let result = core
                .as_mut()
                .ok_or_else(|| "no SNES ROM loaded".to_string())
                .and_then(|core| {
                    core.load_save_ram(&bytes)
                        .map_err(|error| error.to_string())
                });
            send_reply(reply, result);
        }
    }
}

fn send_reply<T>(reply: Sender<Result<T, String>>, result: Result<T, String>) {
    if reply.send(result).is_err() {
        log::warn!("SNES runtime reply send failed");
    }
}

struct Msu1Sidecars {
    data: Option<Vec<u8>>,
    audio_tracks: Vec<u16>,
}

fn load_msu1_sidecars(path: Option<&Path>) -> Result<Msu1Sidecars, String> {
    let Some(path) = path else {
        return Ok(Msu1Sidecars {
            data: None,
            audio_tracks: Vec::new(),
        });
    };
    Ok(Msu1Sidecars {
        data: load_msu1_data_sidecar(path)?,
        audio_tracks: discover_msu1_audio_tracks(path)?,
    })
}

fn load_msu1_data_sidecar(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let data_path = path.with_extension("msu");
    match fs::read(&data_path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "failed to read MSU-1 data sidecar `{}`: {error}",
            data_path.display()
        )),
    }
}

fn discover_msu1_audio_tracks(path: &Path) -> Result<Vec<u16>, String> {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(Vec::new());
    };
    let prefix = format!("{stem}-");
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let entries = fs::read_dir(directory).map_err(|error| {
        format!(
            "failed to scan MSU-1 audio sidecars in `{}`: {error}",
            directory.display()
        )
    })?;
    let mut tracks = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to scan MSU-1 audio sidecars in `{}`: {error}",
                directory.display()
            )
        })?;
        let path = entry.path();
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pcm"))
        {
            continue;
        }
        let Some(file_stem) = path.file_stem().and_then(|file_stem| file_stem.to_str()) else {
            continue;
        };
        if let Some(track) = file_stem
            .strip_prefix(&prefix)
            .and_then(|track| track.parse::<u16>().ok())
        {
            tracks.push(track);
        }
    }
    tracks.sort_unstable();
    tracks.dedup();
    Ok(tracks)
}

fn step_snes_frame(core: &mut Core) -> Result<(), String> {
    let target_cycles = core.cycles().wrapping_add(CPU_CYCLES_PER_FRAME);
    while core.cycles() < target_cycles && core.current_state() != CpuState::Stopped {
        core.step().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn publish_metrics(
    shared: &SnesRuntimeShared,
    frame_counter: u64,
    loaded: bool,
    paused: bool,
    emulation_fps: f32,
) {
    shared.publish_metrics(ConsoleMetrics {
        frame_counter,
        loaded,
        paused,
        emulation_fps,
        speed_multiplier: if emulation_fps > 0.0 {
            emulation_fps / nerust_timer::TARGET_FPS
        } else {
            0.0
        },
    });
}

fn snes_video_profile() -> VideoRenderProfile {
    let logical_size = LogicalSize {
        width: SCREEN_WIDTH,
        height: SCREEN_HEIGHT,
    };
    VideoRenderProfile {
        source_logical_size: logical_size,
        logical_size,
        physical_size: PhysicalSize {
            width: 4.0 * SCREEN_WIDTH as f32 / 3.0,
            height: SCREEN_HEIGHT as f32,
        },
    }
}

fn render_backdrop_frame(core: &Core) -> Vec<u8> {
    let current_inidisp = core.peek(0x002100);
    let current_color0 = u16::from_le_bytes([core.peek_cgram(0), core.peek_cgram(1)]) & 0x7FFF;
    let mut frame = Vec::with_capacity(SCREEN_WIDTH * SCREEN_HEIGHT * 4);
    for y in 0..SCREEN_HEIGHT {
        let line = core.presented_backdrop_line(y);
        let color = line_color(
            line.map_or(current_inidisp, |line| line.inidisp),
            line.map_or(current_color0, |line| line.color0),
        );
        for _ in 0..SCREEN_WIDTH {
            frame.extend_from_slice(&color);
        }
    }
    frame
}

fn opaque_black_frame() -> Vec<u8> {
    let mut frame = vec![0; SCREEN_WIDTH * SCREEN_HEIGHT * 4];
    for pixel in frame.chunks_exact_mut(4) {
        pixel[3] = 0xFF;
    }
    frame
}

fn line_color(inidisp: u8, color: u16) -> [u8; 4] {
    let brightness = inidisp & 0x0F;
    if inidisp & 0x80 != 0 || brightness == 0 {
        [0, 0, 0, 0xFF]
    } else {
        snes_color_to_rgba(color & 0x7FFF, brightness)
    }
}

fn snes_color_to_rgba(color: u16, brightness: u8) -> [u8; 4] {
    let red = scale_channel((color & 0x1F) as u8, brightness);
    let green = scale_channel(((color >> 5) & 0x1F) as u8, brightness);
    let blue = scale_channel(((color >> 10) & 0x1F) as u8, brightness);
    [red, green, blue, 0xFF]
}

fn scale_channel(channel: u8, brightness: u8) -> u8 {
    let expanded = (u16::from(channel) * 255 + 15) / 31;
    ((expanded * u16::from(brightness) + 7) / 15) as u8
}

fn snes_input_topology_descriptor() -> InputTopologyDescriptor {
    InputTopologyDescriptor {
        system: SystemId::Snes,
        ports: vec![PortDescriptor {
            id: SNES_PORT_ONE,
            label: "Controller Port 1",
            attachments: vec![AttachmentSlotDescriptor {
                id: SNES_ATTACHMENT_CONTROLLER_ONE,
                label: "Controller 1",
                device: SNES_STANDARD_PAD,
                supported_devices: vec![SNES_STANDARD_PAD],
            }],
        }],
        devices: vec![DeviceDescriptor {
            kind: SNES_STANDARD_PAD,
            label: "Standard Pad",
            controls: standard_pad_controls(),
        }],
    }
}

fn standard_pad_controls() -> Vec<ControlDescriptor> {
    [
        (SNES_CONTROL_B, "B"),
        (SNES_CONTROL_Y, "Y"),
        (SNES_CONTROL_SELECT, "Select"),
        (SNES_CONTROL_START, "Start"),
        (SNES_CONTROL_UP, "Up"),
        (SNES_CONTROL_DOWN, "Down"),
        (SNES_CONTROL_LEFT, "Left"),
        (SNES_CONTROL_RIGHT, "Right"),
        (SNES_CONTROL_A, "A"),
        (SNES_CONTROL_X, "X"),
        (SNES_CONTROL_L, "L"),
        (SNES_CONTROL_R, "R"),
    ]
    .into_iter()
    .map(|(id, label)| {
        ControlDescriptor::Digital(DigitalControlDescriptor {
            id,
            label,
            description: "",
        })
    })
    .collect()
}

fn snes_control_from_persisted(control: &str) -> Option<DigitalControlId> {
    match control {
        value if value == SNES_CONTROL_B.as_str() => Some(SNES_CONTROL_B),
        value if value == SNES_CONTROL_Y.as_str() => Some(SNES_CONTROL_Y),
        value if value == SNES_CONTROL_SELECT.as_str() => Some(SNES_CONTROL_SELECT),
        value if value == SNES_CONTROL_START.as_str() => Some(SNES_CONTROL_START),
        value if value == SNES_CONTROL_UP.as_str() => Some(SNES_CONTROL_UP),
        value if value == SNES_CONTROL_DOWN.as_str() => Some(SNES_CONTROL_DOWN),
        value if value == SNES_CONTROL_LEFT.as_str() => Some(SNES_CONTROL_LEFT),
        value if value == SNES_CONTROL_RIGHT.as_str() => Some(SNES_CONTROL_RIGHT),
        value if value == SNES_CONTROL_A.as_str() => Some(SNES_CONTROL_A),
        value if value == SNES_CONTROL_X.as_str() => Some(SNES_CONTROL_X),
        value if value == SNES_CONTROL_L.as_str() => Some(SNES_CONTROL_L),
        value if value == SNES_CONTROL_R.as_str() => Some(SNES_CONTROL_R),
        _ => None,
    }
}

fn button_mask(control: DigitalControlId) -> Option<u16> {
    match control {
        value if value == SNES_CONTROL_B => Some(1 << 15),
        value if value == SNES_CONTROL_Y => Some(1 << 14),
        value if value == SNES_CONTROL_SELECT => Some(1 << 13),
        value if value == SNES_CONTROL_START => Some(1 << 12),
        value if value == SNES_CONTROL_UP => Some(1 << 11),
        value if value == SNES_CONTROL_DOWN => Some(1 << 10),
        value if value == SNES_CONTROL_LEFT => Some(1 << 9),
        value if value == SNES_CONTROL_RIGHT => Some(1 << 8),
        value if value == SNES_CONTROL_A => Some(1 << 7),
        value if value == SNES_CONTROL_X => Some(1 << 6),
        value if value == SNES_CONTROL_L => Some(1 << 5),
        value if value == SNES_CONTROL_R => Some(1 << 4),
        _ => None,
    }
}

fn decode_buttons(bytes: &[u8]) -> Result<u16, String> {
    let bytes: [u8; 2] = bytes
        .try_into()
        .map_err(|_| format!("invalid SNES input state length: {}", bytes.len()))?;
    Ok(u16::from_le_bytes(bytes))
}
