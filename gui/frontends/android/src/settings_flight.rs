//! Request ownership independent of JNI, caches, and activity lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(transparent)]
pub(crate) struct SettingsRequestId(pub(crate) u64);

pub(crate) struct SettingsFlight<T> {
    next: u64,
    pending: Option<SettingsRequestId>,
    /// Last begun id, kept across `suspend`. The standalone settings
    /// activity stops MainActivity (surface destroyed → app suspend) by
    /// design, so `suspend` must release the gate for reopening without
    /// invalidating the request the open screen will still deliver.
    last: Option<SettingsRequestId>,
    accepted: Option<T>,
}

impl<T> SettingsFlight<T> {
    pub(crate) const fn new() -> Self {
        Self {
            next: 1,
            pending: None,
            last: None,
            accepted: None,
        }
    }

    pub(crate) fn begin(&mut self) -> Option<SettingsRequestId> {
        if self.pending.is_some() || self.accepted.is_some() {
            return None;
        }
        let id = SettingsRequestId(self.next);
        self.next = self.next.wrapping_add(1).max(1);
        self.pending = Some(id);
        self.last = Some(id);
        Some(id)
    }

    pub(crate) fn accept(&mut self, id: SettingsRequestId, result: T) -> bool {
        // The live request, or the remembered one when no newer request
        // began after a suspend. A consumed id never works twice: `last`
        // clears on success, so replayed/delayed duplicates still fail.
        let known = self.pending == Some(id) || (self.pending.is_none() && self.last == Some(id));
        if !known {
            return false;
        }
        self.pending = None;
        self.last = None;
        self.accepted = Some(result);
        true
    }

    pub(crate) fn suspend(&mut self) {
        self.pending = None;
    }
    pub(crate) fn is_open(&self) -> bool {
        self.pending.is_some()
    }
    pub(crate) fn take(&mut self) -> Option<T> {
        self.accepted.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_save_survives_suspend_and_blocks_overwrite_or_reopen() {
        let mut flight = SettingsFlight::new();
        let id = flight.begin().unwrap();
        assert!(flight.accept(id, "save"));
        flight.suspend();
        assert!(!flight.accept(id, "dismiss"));
        assert!(flight.begin().is_none());
        assert_eq!(flight.take(), Some("save"));
        assert!(flight.begin().is_some());
    }

    #[test]
    fn stale_suspended_callback_never_closes_new_request() {
        let mut flight = SettingsFlight::new();
        let old = flight.begin().unwrap();
        flight.suspend();
        let live = flight.begin().unwrap();
        assert!(!flight.accept(old, "old"));
        assert!(flight.is_open());
        assert!(flight.accept(live, "new"));
        assert_eq!(flight.take(), Some("new"));
    }

    #[test]
    fn standalone_activity_result_survives_cover_suspend() {
        // The production order: begin, MainActivity stops under the opaque
        // settings activity (surface destroyed → app suspend clears the
        // gate), then the still-open screen delivers its result. Dropping
        // that result silently discarded every settings save.
        let mut flight = SettingsFlight::new();
        let id = flight.begin().unwrap();
        flight.suspend();
        assert!(!flight.is_open(), "suspend releases the reopen gate");
        assert!(flight.accept(id, "save"));
        assert!(
            !flight.accept(id, "save"),
            "a consumed id never works twice"
        );
        assert_eq!(flight.take(), Some("save"));
    }

    #[test]
    fn consumed_id_never_applies_to_later_request() {
        let mut flight = SettingsFlight::new();
        let first = flight.begin().unwrap();
        flight.suspend();
        assert!(flight.accept(first, "first"));
        assert_eq!(flight.take(), Some("first"));
        let second = flight.begin().unwrap();
        assert!(!flight.accept(first, "first"), "no replay across requests");
        assert!(flight.accept(second, "second"));
        assert_eq!(flight.take(), Some("second"));
    }
}
