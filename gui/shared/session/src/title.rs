use nerust_console::ConsoleMetrics;

pub fn window_title(paused: bool, console_metrics: ConsoleMetrics) -> String {
    let state = if paused { "Nes -- Paused" } else { "Nes" };
    let mut title = if console_metrics.loaded {
        format!(
            "{state} | FPS {:.1} | Speed x{:.2}",
            console_metrics.emulation_fps, console_metrics.speed_multiplier
        )
    } else {
        format!("{state} | No ROM")
    };
    if let Some(warning) = console_metrics.runtime_warnings.title_summary() {
        title.push_str(" | Warning: ");
        title.push_str(warning);
    }
    title
}

#[cfg(test)]
mod tests {
    use super::window_title;
    use nerust_console::ConsoleMetrics;

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
    fn window_title_surfaces_runtime_warnings() {
        let title = window_title(
            false,
            ConsoleMetrics {
                loaded: true,
                runtime_warnings: nerust_console::ConsoleRuntimeWarnings {
                    snes_audio_unsupported: true,
                    snes_renderer_fallback: true,
                },
                ..ConsoleMetrics::default()
            },
        );

        assert!(title.contains("Warning: SNES audio unavailable; renderer fallback"));
    }
}
