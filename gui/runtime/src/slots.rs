use nerust_persistence::{model::StateSlotSummary, time::format_slot_saved_at};

pub fn slot_label(slot: &StateSlotSummary, active_slot: Option<u64>) -> String {
    let saved_at = format_slot_saved_at(slot.saved_at);
    let active = if active_slot == Some(slot.slot_id) {
        " (active)"
    } else {
        ""
    };
    format!("Slot {} — {saved_at}{active}", slot.slot_id)
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        time::{Duration, UNIX_EPOCH},
    };

    use nerust_persistence::model::StateSlotSummary;

    use super::slot_label;

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
}
