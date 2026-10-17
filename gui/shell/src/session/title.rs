use nerust_emu_thread::ConsoleMetrics;

pub fn window_title(
    paused: bool,
    console_metrics: ConsoleMetrics,
    system_name: Option<&str>,
    active_slot: Option<u64>,
    slot_count: usize,
) -> String {
    let label = system_name.unwrap_or("");
    let state = if paused {
        format!("{label} -- Paused")
    } else {
        label.to_string()
    };
    let slot = match active_slot {
        Some(id) => format!(" | Slot {id} ({slot_count} slots)"),
        None if slot_count > 0 => format!(" | {slot_count} slots"),
        None => String::new(),
    };
    if console_metrics.loaded {
        // Runtime metrics live in the status readout, not here:
        // fps duplicates the OSD status pill and speed is noise.
        format!("{state}{slot}")
    } else {
        format!("{state}{slot} | No ROM")
    }
}

#[cfg(test)]
mod tests {
    use nerust_emu_thread::ConsoleMetrics;

    use super::window_title;

    #[test]
    fn window_title_omits_runtime_metrics() {
        let title = window_title(
            false,
            ConsoleMetrics {
                loaded: true,
                emulation_fps: 59.9,
                speed_multiplier: 1.01,
                ..ConsoleMetrics::default()
            },
            Some("Nes"),
            Some(2),
            3,
        );

        assert!(!title.contains("FPS"));
        assert!(!title.contains("Speed"));
        assert!(title.contains("Slot 2"));
        assert!(title.contains("3 slots"));
    }

    #[test]
    fn window_title_marks_no_rom() {
        assert!(
            window_title(true, ConsoleMetrics::default(), Some("Nes"), None, 0).contains("Paused")
        );
        assert!(
            window_title(true, ConsoleMetrics::default(), Some("Nes"), None, 0).contains("No ROM")
        );
    }

    #[test]
    fn window_title_surfaces_slot_count_without_active_slot() {
        let title = window_title(false, ConsoleMetrics::default(), Some("Nes"), None, 2);
        assert!(title.contains("2 slots"));
        assert!(!title.contains("Slot"));
    }
}
