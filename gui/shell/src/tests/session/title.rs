use super::window_title;
use nerust_emu_thread::ConsoleMetrics;

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
