//! Request ownership independent of JNI, caches, and activity lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(transparent)]
pub(crate) struct SettingsRequestId(pub(crate) u64);

pub(crate) struct SettingsFlight<T> {
    next: u64,
    pending: Option<SettingsRequestId>,
    accepted: Option<T>,
}

impl<T> SettingsFlight<T> {
    pub(crate) const fn new() -> Self {
        Self {
            next: 1,
            pending: None,
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
        Some(id)
    }

    pub(crate) fn accept(&mut self, id: SettingsRequestId, result: T) -> bool {
        if self.pending != Some(id) {
            return false;
        }
        self.pending = None;
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
}
