/// Window-title inputs as one struct. Positional bools at every call
/// site never said which flag was which; the struct fields do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleInput<'a> {
    pub paused: bool,
    pub loaded: bool,
    pub system_name: Option<&'a str>,
    pub active_slot: Option<u64>,
    pub slot_count: usize,
}

/// Stable state key for title caching: pause/load quadrants as one static.
/// Frontends key their own caches on this instead of formatting first.
pub fn title_state_key(paused: bool, loaded: bool) -> &'static str {
    match (paused, loaded) {
        (true, true) => "paused-loaded",
        (true, false) => "paused-unloaded",
        (false, true) => "running-loaded",
        (false, false) => "running-unloaded",
    }
}

/// Single title builder: structured input in, display text out.
/// All title entry points delegate here.
pub fn title(input: TitleInput<'_>) -> String {
    let label = input.system_name.unwrap_or("");
    let state = if input.paused {
        format!("{label} -- Paused")
    } else {
        label.to_string()
    };
    let slot = match input.active_slot {
        Some(id) => format!(" | Slot {id} ({count} slots)", count = input.slot_count),
        None if input.slot_count > 0 => {
            format!(" | {count} slots", count = input.slot_count)
        }
        None => String::new(),
    };
    if input.loaded {
        // Runtime metrics live in the status readout, not here:
        // fps duplicates the OSD status pill and speed is noise.
        format!("{state}{slot}")
    } else {
        format!("{state}{slot} | No ROM")
    }
}

pub fn window_title(
    paused: bool,
    loaded: bool,
    system_name: Option<&str>,
    active_slot: Option<u64>,
    slot_count: usize,
) -> String {
    window_title_input(TitleInput {
        paused,
        loaded,
        system_name,
        active_slot,
        slot_count,
    })
}

/// Structured form of [`window_title`]: single source for the text,
/// so positional call sites cannot misalign the flags.
pub fn window_title_input(input: TitleInput<'_>) -> String {
    title(input)
}

/// Cache key for [`TitleCache`]: everything the title text reads.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct TitleKey {
    paused: bool,
    loaded: bool,
    system_name: Option<String>,
    active_slot: Option<u64>,
    slot_count: usize,
}

impl<'a> From<&TitleInput<'a>> for TitleKey {
    fn from(input: &TitleInput<'a>) -> Self {
        Self {
            paused: input.paused,
            loaded: input.loaded,
            system_name: input.system_name.map(str::to_string),
            active_slot: input.active_slot,
            slot_count: input.slot_count,
        }
    }
}

/// Per-frame title cache: rebuilds only when the key changes,
/// otherwise serves the stored string without formatting.
#[derive(Debug, Default)]
pub struct TitleCache {
    last_key: Option<TitleKey>,
    last_title: String,
}

impl TitleCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrowed title for `input`: formats only on key change.
    pub fn title(&mut self, input: TitleInput<'_>) -> &str {
        let key = TitleKey::from(&input);
        if self.last_key.as_ref() != Some(&key) {
            self.last_title = title(input);
            self.last_key = Some(key);
        }
        &self.last_title
    }
}

#[cfg(test)]
mod tests {
    use super::{TitleCache, TitleInput, window_title, window_title_input};

    #[test]
    fn window_title_omits_runtime_metrics() {
        let title = window_title(false, true, Some("Nes"), Some(2), 3);

        assert!(!title.contains("FPS"));
        assert!(!title.contains("Speed"));
        assert!(title.contains("Slot 2"));
        assert!(title.contains("3 slots"));
    }

    #[test]
    fn window_title_marks_no_rom() {
        assert!(window_title(true, false, Some("Nes"), None, 0).contains("Paused"));
        assert!(window_title(true, false, Some("Nes"), None, 0).contains("No ROM"));
    }

    #[test]
    fn window_title_surfaces_slot_count_without_active_slot() {
        let title = window_title(false, false, Some("Nes"), None, 2);
        assert!(title.contains("2 slots"));
        assert!(!title.contains("Slot"));
    }

    #[test]
    fn title_input_matches_positional_form() {
        let input = TitleInput {
            paused: true,
            loaded: true,
            system_name: Some("Nes"),
            active_slot: Some(2),
            slot_count: 3,
        };
        assert_eq!(
            window_title_input(input),
            window_title(true, true, Some("Nes"), Some(2), 3)
        );
    }

    #[test]
    fn title_cache_rebuilds_only_on_key_change() {
        let mut cache = TitleCache::new();
        let input = || TitleInput {
            paused: false,
            loaded: true,
            system_name: Some("Nes"),
            active_slot: Some(1),
            slot_count: 2,
        };
        let first = cache.title(input()).to_string();
        // Same key: no rebuild, same borrow.
        assert_eq!(cache.title(input()), first);
        // Changed key: fresh text.
        let changed = TitleInput {
            paused: true,
            ..input()
        };
        assert_ne!(cache.title(changed), first);
    }
}
