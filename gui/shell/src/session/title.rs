/// Window-title inputs as one struct. Positional bools at every call
/// site never said which flag was which; the struct fields do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleInput {
    pub paused: bool,
    pub loaded: bool,
    /// Factory-static display name: no per-frame allocation in the cache key.
    pub system_name: Option<&'static str>,
    pub active_slot: Option<u64>,
    pub slot_count: usize,
}

/// Single title builder: structured input in, display text out.
/// All title entry points delegate here.
pub fn title(input: TitleInput) -> String {
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

/// Cache key for [`TitleCache`]: everything the title text reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct TitleKey {
    paused: bool,
    loaded: bool,
    system_name: Option<&'static str>,
    active_slot: Option<u64>,
    slot_count: usize,
}

impl From<&TitleInput> for TitleKey {
    fn from(input: &TitleInput) -> Self {
        Self {
            paused: input.paused,
            loaded: input.loaded,
            system_name: input.system_name,
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

    /// Borrowed title for `input`: formats only on key change, never allocating for the key.
    pub fn title(&mut self, input: TitleInput) -> &str {
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
    use super::{TitleCache, TitleInput, title};

    #[test]
    fn title_omits_runtime_metrics() {
        let text = title(TitleInput {
            paused: false,
            loaded: true,
            system_name: Some("Nes"),
            active_slot: Some(2),
            slot_count: 3,
        });

        assert!(!text.contains("FPS"));
        assert!(!text.contains("Speed"));
        assert!(text.contains("Slot 2"));
        assert!(text.contains("3 slots"));
    }

    #[test]
    fn title_marks_no_rom() {
        let text = title(TitleInput {
            paused: true,
            loaded: false,
            system_name: Some("Nes"),
            active_slot: None,
            slot_count: 0,
        });
        assert!(text.contains("Paused"));
        assert!(text.contains("No ROM"));
    }

    #[test]
    fn title_surfaces_slot_count_without_active_slot() {
        let text = title(TitleInput {
            paused: false,
            loaded: false,
            system_name: Some("Nes"),
            active_slot: None,
            slot_count: 2,
        });
        assert!(text.contains("2 slots"));
        assert!(!text.contains("Slot"));
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
        assert!(!first.contains("No ROM"));
        // Changed key: fresh text.
        let changed = TitleInput {
            paused: true,
            ..input()
        };
        assert_ne!(cache.title(changed), first);
        // Loaded flip with no ROM queued: the cached line names it.
        let unloaded = TitleInput {
            loaded: false,
            active_slot: None,
            slot_count: 0,
            ..input()
        };
        assert!(cache.title(unloaded).contains("No ROM"));
    }
}

#[cfg(test)]
mod title_cov_tests {
    use crate::session::title::*;

    #[test]
    fn cached_and_direct_titles_agree() {
        use crate::session::osd::TickMetrics;
        let tick = TickMetrics {
            paused: true,
            loaded: true,
            fps: 60.0,
            frame_counter: 7,
        };
        let input = TitleInput {
            paused: tick.paused,
            loaded: tick.loaded,
            system_name: Some("Nes"),
            active_slot: Some(2),
            slot_count: 3,
        };
        let mut cache = TitleCache::new();
        assert_eq!(cache.title(input), title(input));
    }
}
