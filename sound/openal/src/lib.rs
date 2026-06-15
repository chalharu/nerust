use alto::*;
use nerust_sound_traits::{MixerInput, Sound};
#[cfg(target_os = "macos")]
use std::os::unix::process::CommandExt;
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::sync::Once;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;
use std::{f64, thread};

#[cfg(any(target_os = "macos", test))]
const DYLD_ENV_VARS: [&str; 2] = ["DYLD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH"];
#[cfg(any(target_os = "macos", test))]
const MACOS_RUNTIME_SANITIZED_ENV: &str = "NERUST_MACOS_RUNTIME_SANITIZED";

#[cfg(target_os = "macos")]
const MACOS_OPENAL_CANDIDATES: &[&str] = &[
    "/System/Library/Frameworks/OpenAL.framework/OpenAL",
    "/System/Library/Frameworks/OpenAL.framework/Versions/Current/OpenAL",
    "/System/Library/Frameworks/OpenAL.framework/Versions/A/OpenAL",
];

#[cfg(target_os = "macos")]
static PREPARE_MACOS_RUNTIME_ONCE: Once = Once::new();

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
enum MacosRuntimeAction {
    Continue,
    Reexec,
    Abort,
}

#[cfg(any(target_os = "macos", test))]
fn macos_runtime_action(
    guard_present: bool,
    dyld_env_vars_present: &[&'static str],
) -> MacosRuntimeAction {
    match (guard_present, dyld_env_vars_present.is_empty()) {
        (_, true) => MacosRuntimeAction::Continue,
        (false, false) => MacosRuntimeAction::Reexec,
        (true, false) => MacosRuntimeAction::Abort,
    }
}

#[cfg(any(target_os = "macos", test))]
fn present_dyld_env_vars(mut is_present: impl FnMut(&str) -> bool) -> Vec<&'static str> {
    DYLD_ENV_VARS
        .into_iter()
        .filter(|name| is_present(name))
        .collect()
}

pub fn prepare_macos_runtime() {
    #[cfg(target_os = "macos")]
    PREPARE_MACOS_RUNTIME_ONCE.call_once(|| {
        let guard_present = std::env::var_os(MACOS_RUNTIME_SANITIZED_ENV).is_some();
        let dyld_env_vars_present = present_dyld_env_vars(|name| std::env::var_os(name).is_some());

        match macos_runtime_action(guard_present, &dyld_env_vars_present) {
            MacosRuntimeAction::Continue => clear_macos_runtime_guard(),
            MacosRuntimeAction::Reexec => reexec_process_without_dyld_env(&dyld_env_vars_present),
            MacosRuntimeAction::Abort => {
                log::error!(
                    "{MACOS_RUNTIME_SANITIZED_ENV} is set but macOS DYLD environment variables are still present ({vars}); refusing to continue",
                    vars = dyld_env_vars_present.join(", "),
                );
                std::process::exit(1);
            }
        }
    });
}

#[cfg(target_os = "macos")]
fn clear_macos_runtime_guard() {
    if std::env::var_os(MACOS_RUNTIME_SANITIZED_ENV).is_some() {
        // SAFETY: This runs during process startup on the main thread before any frontend or audio
        // worker threads are created, so there is no concurrent environment mutation.
        unsafe {
            std::env::remove_var(MACOS_RUNTIME_SANITIZED_ENV);
        }
    }
}

#[cfg(target_os = "macos")]
fn reexec_process_without_dyld_env(dyld_env_vars_present: &[&'static str]) -> ! {
    log::debug!(
        "re-executing process without macOS DYLD environment variables to avoid ImageIO/AppKit plugin conflicts: {}",
        dyld_env_vars_present.join(", "),
    );

    let mut command = Command::new(
        std::env::current_exe().expect("failed to resolve current executable for macOS re-exec"),
    );
    command.args(std::env::args_os().skip(1));
    command.env(MACOS_RUNTIME_SANITIZED_ENV, "1");
    for name in DYLD_ENV_VARS {
        command.env_remove(name);
    }
    let err = command.exec();

    log::error!("failed to re-execute process without DYLD environment variables: {err}");
    std::process::exit(1);
}

#[derive(Debug)]
struct FadeBuffer {
    data_receiver: Receiver<f32>,
    fade_width: usize,
    fadein_window_lut: Vec<f32>,
    fadeout_window_lut: Vec<f32>,
    fade_buffer: Vec<f32>,
    input_pos: usize,
    output_pos: usize,
    fade_pos: usize,
}

impl FadeBuffer {
    pub(crate) fn new(data_receiver: Receiver<f32>, fade_width: usize) -> Self {
        // 必ず lut[0] = 0 とする
        let hannning_fadein_window_lut = (0..fade_width)
            .map(|x| 0.5 - ((x as f64 * f64::consts::PI / fade_width as f64).cos() * 0.5) as f32)
            .collect::<Vec<_>>();
        // 必ず lut[0] = 1 とする
        let hannning_fadeout_window_lut = (0..fade_width)
            .map(|x| {
                0.5 - (((x as f64 + fade_width as f64) * f64::consts::PI / fade_width as f64).cos()
                    * 0.5) as f32
            })
            .collect::<Vec<_>>();
        let fade_buffer = vec![0.0; (fade_width * 2 + 1).next_power_of_two()];

        Self {
            data_receiver,
            fade_width,
            fadein_window_lut: hannning_fadein_window_lut,
            fadeout_window_lut: hannning_fadeout_window_lut,
            fade_buffer,
            input_pos: 0,
            output_pos: 0,
            fade_pos: 0,
        }
    }
}

impl Iterator for FadeBuffer {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let lenmask = self.fade_buffer.len() - 1;
        while (self.output_pos + self.fade_width) & lenmask != self.input_pos {
            if let Ok(data) = self.data_receiver.try_recv() {
                self.fade_buffer[self.input_pos] = data;
                self.input_pos = (self.input_pos + 1) & lenmask;
            } else {
                break;
            }
        }
        Some(
            if (self.output_pos + self.fade_width) & lenmask != self.input_pos || self.fade_pos > 0
            {
                // 入力データが不足している場合
                self.fade_pos += 1;
                if self.fade_pos == self.fade_width {
                    self.fade_pos = 0;
                    self.fade_buffer[self.output_pos]
                } else {
                    let fade_pos = (self.fade_buffer.len() + self.output_pos + self.fade_pos
                        - self.fade_width)
                        & lenmask;
                    let out_pos = (self.output_pos + self.fade_pos) & lenmask;
                    self.fade_buffer[fade_pos] * self.fadein_window_lut[self.fade_pos]
                        + self.fade_buffer[out_pos] * self.fadeout_window_lut[self.fade_pos]
                }
            } else {
                self.output_pos = (self.output_pos + 1) & lenmask;
                self.fade_buffer[self.output_pos]
            },
        )
    }
}

struct OpenAlState {
    // alto: Option<Alto>,
    // dev: Option<OutputDevice>,
    // ctx: Option<Context>,
    src: Option<StreamingSource>,
    playing_receiver: Receiver<bool>,
    sample_rate: i32,
    playing: bool,
    fade_buffer: FadeBuffer,
    buffer: Vec<Mono<i16>>,
    buffer_count: usize,
}

impl OpenAlState {
    fn create_context(dev: &OutputDevice, requested_sample_rate: i32) -> Result<Context, String> {
        let attrs = ContextAttrs {
            frequency: Some(requested_sample_rate),
            ..ContextAttrs::default()
        };
        match dev.new_context(Some(attrs)) {
            Ok(ctx) => Ok(ctx),
            Err(requested_err) => {
                log::warn!(
                    "failed to create OpenAL context at {requested_sample_rate} Hz ({requested_err:?}); retrying with backend defaults"
                );
                dev.new_context(None).map_err(|default_err| {
                    format!(
                        "failed to create OpenAL context (requested {requested_sample_rate} Hz: {requested_err:?}; default attributes: {default_err:?})"
                    )
                })
            }
        }
    }

    fn resolve_playback_sample_rate(
        alto: &Alto,
        dev: &OutputDevice,
        requested_sample_rate: i32,
    ) -> i32 {
        let mut actual_sample_rate = 0;
        unsafe {
            alto.raw_api().alcGetIntegerv(
                dev.as_raw(),
                sys::ALC_FREQUENCY,
                1,
                &mut actual_sample_rate,
            );
        }
        match alto.get_error(dev.as_raw()) {
            Ok(()) if actual_sample_rate > 0 => {
                if actual_sample_rate != requested_sample_rate {
                    log::debug!(
                        "OpenAL playback sample rate resolved to {actual_sample_rate} Hz instead of requested {requested_sample_rate} Hz"
                    );
                }
                actual_sample_rate
            }
            Ok(()) => {
                log::warn!(
                    "OpenAL reported a non-positive playback sample rate ({actual_sample_rate}); using requested {requested_sample_rate} Hz"
                );
                requested_sample_rate
            }
            Err(err) => {
                log::warn!(
                    "failed to query OpenAL playback sample rate ({err:?}); using requested {requested_sample_rate} Hz"
                );
                requested_sample_rate
            }
        }
    }

    fn load_alto() -> Result<Alto, String> {
        let mut errors = Vec::new();

        match Alto::load_default() {
            Ok(alto) => {
                log::info!("loaded OpenAL with the default loader");
                return Ok(alto);
            }
            Err(err) => errors.push(format!("default loader failed: {err:?}")),
        }

        #[cfg(target_os = "macos")]
        for path in MACOS_OPENAL_CANDIDATES {
            match Alto::load(path) {
                Ok(alto) => {
                    log::info!("loaded OpenAL from {path}");
                    return Ok(alto);
                }
                Err(err) => errors.push(format!("{path}: {err:?}")),
            }
        }

        Err(format!("failed to load OpenAL: {}", errors.join(" | ")))
    }

    fn create_streaming_source(
        requested_sample_rate: i32,
        buffer_width: usize,
        buffer_count: usize,
    ) -> Result<(StreamingSource, i32), String> {
        let alto = Self::load_alto()?;
        let dev = alto
            .open(None)
            .map_err(|err| format!("failed to open default OpenAL output device: {err:?}"))?;
        let mut ctx = Self::create_context(&dev, requested_sample_rate)?;
        let playback_sample_rate =
            Self::resolve_playback_sample_rate(&alto, &dev, requested_sample_rate);
        let mut src = ctx
            .new_streaming_source()
            .map_err(|err| format!("failed to create OpenAL streaming source: {err:?}"))?;
        for _ in 0..buffer_count {
            Self::add_buffer(&mut ctx, &mut src, playback_sample_rate, buffer_width)
                .map_err(|err| format!("failed to queue initial OpenAL buffer: {err:?}"))?;
        }
        Ok((src, playback_sample_rate))
    }

    pub(crate) fn new(
        sample_rate: i32,
        buffer_width: usize,
        buffer_count: usize,
        playing_receiver: Receiver<bool>,
        data_receiver: Receiver<f32>,
        fade_width: usize,
        src: Option<StreamingSource>,
    ) -> Self {
        Self {
            src,
            sample_rate,
            playing: false,
            playing_receiver,
            fade_buffer: FadeBuffer::new(data_receiver, fade_width),
            buffer: vec![Mono { center: 0 }; buffer_width],
            buffer_count,
        }
    }

    fn add_buffer(
        ctx: &mut Context,
        src: &mut StreamingSource,
        sample_rate: i32,
        buffer_width: usize,
    ) -> AltoResult<()> {
        let data = vec![Mono { center: 0_i16 }; buffer_width];
        let buf = ctx.new_buffer(&data, sample_rate)?;
        src.queue_buffer(buf)?;
        Ok(())
    }

    fn fill_buffer(
        src: &mut StreamingSource,
        sample_rate: i32,
        fade_buffer: &mut FadeBuffer,
        buffer: &mut Vec<Mono<i16>>,
    ) -> Result<(), String> {
        let mut buf = match src.unqueue_buffer() {
            Ok(b) => b,
            Err(err) => return Err(format!("OpenAL unqueue_buffer failed: {err:?}")),
        };
        let len = buffer.len();
        buffer.clear();
        for d in fade_buffer.take(len) {
            buffer.push(Mono {
                center: (d * f32::from(i16::MAX)) as i16,
            });
        }
        if let Err(err) = buf.set_data(buffer, sample_rate) {
            return Err(format!("OpenAL buffer set_data failed: {err:?}"));
        }
        if let Err(err) = src.queue_buffer(buf) {
            return Err(format!("OpenAL queue_buffer failed: {err:?}"));
        }
        Ok(())
    }

    fn step(&mut self) {
        if let Ok(new_playing) = self.playing_receiver.try_recv() {
            self.playing = new_playing;
        }

        // If there's no streaming source, try to create one
        if self.src.is_none() {
            match Self::create_streaming_source(
                self.sample_rate,
                self.buffer.len(),
                self.buffer_count,
            ) {
                Ok((new_src, playback_rate)) => {
                    let old_rate = self.sample_rate;
                    self.src = Some(new_src);
                    if playback_rate != old_rate {
                        self.sample_rate = playback_rate;
                        log::info!(
                            "OpenAL playback rate resolved to {playback_rate} Hz (was {old_rate})"
                        );
                    }
                }
                Err(err) => {
                    // Reinitialization failed; try again later
                    log::debug!("OpenAL reinitialization attempt failed: {err}");
                }
            }
        }

        let mut drop_src = false;

        if let Some(ref mut src) = self.src.as_mut() {
            if self.playing {
                let buffers_processed = src.buffers_processed();
                for _ in 0..buffers_processed {
                    if let Err(err) = Self::fill_buffer(
                        src,
                        self.sample_rate,
                        &mut self.fade_buffer,
                        &mut self.buffer,
                    ) {
                        log::warn!(
                            "OpenAL audio worker encountered error while filling buffer: {err}; dropping streaming source and will attempt reinitialize"
                        );
                        drop_src = true;
                        break;
                    }
                }
                if !drop_src {
                    match src.state() {
                        SourceState::Playing => (),
                        _ => {
                            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| src.play()))
                                .is_err()
                            {
                                log::warn!(
                                    "OpenAL source play panicked; dropping streaming source"
                                );
                                drop_src = true;
                            }
                        }
                    }
                }
            } else if matches!(src.state(), SourceState::Playing)
                && std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| src.pause())).is_err()
            {
                log::warn!("OpenAL source pause panicked; dropping streaming source");
                drop_src = true;
            }
        }

        if drop_src {
            self.src = None;
        }
    }
}

#[derive(Debug)]
pub struct OpenAl {
    stop_sender: Sender<()>,
    playing_sender: Sender<bool>,
    data_sender: Sender<f32>,
    thread: Option<JoinHandle<()>>,
    playback_sample_rate: u32,
}

impl OpenAl {
    pub fn new(sample_rate: i32, buffer_width: usize, buffer_count: usize) -> Self {
        Self::with_gain(sample_rate, buffer_width, buffer_count, 1.0)
    }

    pub fn with_gain(
        sample_rate: i32,
        buffer_width: usize,
        buffer_count: usize,
        _gain: f32,
    ) -> Self {
        let requested_playback_sample_rate = sample_rate;
        let (src, playback_sample_rate) =
            match OpenAlState::create_streaming_source(sample_rate, buffer_width, buffer_count) {
                Ok((src, playback_sample_rate)) => (Some(src), playback_sample_rate),
                Err(err) => {
                    log::error!("{err}");
                    (None, requested_playback_sample_rate)
                }
            };
        let playback_sample_rate_u32 = u32::try_from(playback_sample_rate)
            .expect("OpenAL playback sample_rate must be non-negative");
        let (playing_sender, playing_recv) = channel();
        let (data_sender, data_recv) = channel();
        let (stop_sender, stop_recv) = channel();
        // On macOS, loading Apple's deprecated OpenAL framework from a background thread can
        // race with AppKit/ImageIO initialization. Initialize the backend on the caller thread
        // first, then hand the fully created streaming source to the audio thread.
        let thread = thread::spawn(move || {
            let mut state = OpenAlState::new(
                playback_sample_rate,
                buffer_width,
                buffer_count,
                playing_recv,
                data_recv,
                buffer_width,
                src,
            );
            while stop_recv.try_recv().is_err() {
                state.step();
                thread::sleep(Duration::from_millis(1));
            }
        });

        Self {
            playing_sender,
            data_sender,
            stop_sender,
            thread: Some(thread),
            playback_sample_rate: playback_sample_rate_u32,
        }
    }
}

impl Sound for OpenAl {
    fn pause(&mut self) {
        if self.playing_sender.send(false).is_err() {
            log::warn!("OpenAL channel (playing) send failed");
        }
    }

    fn start(&mut self) {
        if self.playing_sender.send(true).is_err() {
            log::warn!("OpenAL channel (playing) send failed");
        }
    }
}

impl MixerInput for OpenAl {
    fn push(&mut self, data: f32) {
        if self.data_sender.send(data).is_err() {
            log::warn!("OpenAL channel (data) send failed");
        }
    }

    fn sample_rate(&self) -> u32 {
        self.playback_sample_rate
    }
}

impl nerust_contract_core::audio::AudioBackend for OpenAl {
    fn start(&mut self) {
        Sound::start(self);
    }

    fn pause(&mut self) {
        Sound::pause(self);
    }

    fn push(&mut self, data: f32) {
        MixerInput::push(self, data);
    }

    fn sample_rate(&self) -> u32 {
        MixerInput::sample_rate(self)
    }
}

impl Drop for OpenAl {
    fn drop(&mut self) {
        if self.stop_sender.send(()).is_err() {
            log::warn!("OpenAL channel (stop) send failed");
        }
        let _ = self.thread.take().map(JoinHandle::join);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DYLD_ENV_VARS, MACOS_RUNTIME_SANITIZED_ENV, MacosRuntimeAction, macos_runtime_action,
        present_dyld_env_vars,
    };

    #[test]
    fn present_dyld_env_vars_collects_present_variables() {
        let dyld_env_vars_present = present_dyld_env_vars(|name| {
            matches!(
                name,
                MACOS_RUNTIME_SANITIZED_ENV | "DYLD_FALLBACK_LIBRARY_PATH"
            )
        });

        assert_eq!(dyld_env_vars_present, vec![DYLD_ENV_VARS[1]]);
    }

    #[test]
    fn macos_runtime_reexecs_when_dyld_env_is_present_without_guard() {
        assert_eq!(
            macos_runtime_action(false, &[DYLD_ENV_VARS[0]]),
            MacosRuntimeAction::Reexec
        );
    }

    #[test]
    fn macos_runtime_continues_once_reexec_has_cleared_dyld_env() {
        assert_eq!(
            macos_runtime_action(true, &[]),
            MacosRuntimeAction::Continue
        );
    }

    #[test]
    fn macos_runtime_aborts_when_guard_is_set_but_dyld_env_remains() {
        assert_eq!(
            macos_runtime_action(true, &DYLD_ENV_VARS),
            MacosRuntimeAction::Abort
        );
    }
}
