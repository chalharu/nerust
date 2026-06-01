use nerust_persistence::model::StateSlotSummary;
use nerust_persistence::time::format_slot_saved_at;

pub fn slot_label(slot: &StateSlotSummary, active_slot: Option<u64>) -> String {
    let saved_at = format_slot_saved_at(slot.saved_at);
    let active = if active_slot == Some(slot.slot_id) {
        " (active)"
    } else {
        ""
    };
    format!("Slot {} — {saved_at}{active}", slot.slot_id)
}

pub(crate) fn adjacent_slot_id(
    slots: &[StateSlotSummary],
    active_slot: Option<u64>,
    forward: bool,
) -> Option<u64> {
    if slots.is_empty() {
        return None;
    }
    Some(
        if let Some(current) = active_slot
            && let Some(index) = slots.iter().position(|slot| slot.slot_id == current)
        {
            let offset = if forward {
                (index + 1) % slots.len()
            } else {
                (index + slots.len() - 1) % slots.len()
            };
            slots[offset].slot_id
        } else if forward {
            slots[0].slot_id
        } else {
            slots[slots.len() - 1].slot_id
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{adjacent_slot_id, slot_label};
    use nerust_persistence::model::StateSlotSummary;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    fn slot(slot_id: u64) -> StateSlotSummary {
        StateSlotSummary {
            schema_version: 1,
            slot_id,
            path: PathBuf::from(format!("slot-{slot_id}.nst")),
            saved_at: UNIX_EPOCH + Duration::from_secs(1_700_000_000 + slot_id),
            has_thumbnail: false,
            emulator_version: "test".into(),
        }
    }

    #[test]
    fn slot_label_marks_active_slot() {
        let label = slot_label(&slot(2), Some(2));
        assert!(label.contains("Slot 2"));
        assert!(label.contains("(active)"));
    }

    #[test]
    fn adjacent_slot_selection_wraps_in_both_directions() {
        let slots = vec![slot(1), slot(3), slot(7)];

        assert_eq!(adjacent_slot_id(&slots, Some(7), true), Some(1));
        assert_eq!(adjacent_slot_id(&slots, Some(1), false), Some(7));
        assert_eq!(adjacent_slot_id(&slots, None, true), Some(1));
        assert_eq!(adjacent_slot_id(&slots, None, false), Some(7));
    }
}
