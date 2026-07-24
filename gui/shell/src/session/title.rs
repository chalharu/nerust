use nerust_emu_thread::ConsoleMetrics;

pub fn window_title(
    paused: bool,
    console_metrics: ConsoleMetrics,
    system_name: Option<&str>,
) -> String {
    let label = system_name.unwrap_or("");
    let state = if paused {
        format!("{label} -- Paused")
    } else {
        label.to_string()
    };
    if console_metrics.loaded {
        format!(
            "{state} | FPS {:.1} | Speed x{:.2}",
            console_metrics.emulation_fps, console_metrics.speed_multiplier
        )
    } else {
        format!("{state} | No ROM")
    }
}

#[cfg(test)]
mod tests {
    use nerust_emu_thread::ConsoleMetrics;

    use super::window_title;

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
            Some("Nes"),
        );

        assert!(title.contains("FPS 59.9"));
        assert!(title.contains("Speed x1.01"));
    }

    #[test]
    fn window_title_marks_no_rom() {
        assert!(window_title(true, ConsoleMetrics::default(), Some("Nes")).contains("Paused"));
        assert!(window_title(true, ConsoleMetrics::default(), Some("Nes")).contains("No ROM"));
    }
}
