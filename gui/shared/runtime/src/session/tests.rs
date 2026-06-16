use super::{GuiSession, commands::redraw_needed_after_pause_change};
use nerust_console::{Console, ConsoleMetrics};
use nerust_gui_session::core::SessionCore;
use nerust_gui_session::title::window_title;
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_sound_traits::{MixerInput, Sound};

#[derive(Default)]
struct TestSpeaker;

impl Sound for TestSpeaker {
    fn start(&mut self) {}

    fn pause(&mut self) {}
}

impl MixerInput for TestSpeaker {
    fn push(&mut self, _: f32) {}
}

fn test_session() -> GuiSession {
    GuiSession::from_session_core(SessionCore::from_console(Console::new(
        TestSpeaker,
        ScreenBuffer::new_nes_gpu_default(),
        Box::new(nerust_input_nes_runtime::StandardController::new()),
    )))
}

#[test]
fn window_title_surfaces_runtime_metrics() {
    let title = window_title(
        false,
        ConsoleMetrics {
            loaded: true,
            emulation_fps: 59.9,
            speed_multiplier: 1.01,
            ..ConsoleMetrics::default()
        },
    );

    assert!(title.contains("FPS 59.9"));
    assert!(title.contains("Speed x1.01"));
}

#[test]
fn window_title_marks_no_rom() {
    assert!(window_title(true, ConsoleMetrics::default()).contains("Paused"));
    assert!(window_title(true, ConsoleMetrics::default()).contains("No ROM"));
}

#[test]
fn redraw_is_only_requested_when_a_command_resumes_emulation() {
    assert!(redraw_needed_after_pause_change(true, true, false));
    assert!(!redraw_needed_after_pause_change(true, false, false));
    assert!(!redraw_needed_after_pause_change(true, true, true));
    assert!(!redraw_needed_after_pause_change(false, true, false));
}

#[test]
fn test_session_builds_gui_session() {
    let session = test_session();

    assert!(!session.loaded());
    assert!(session.paused());
    assert!(session.window_size().width > 0.0);
}
