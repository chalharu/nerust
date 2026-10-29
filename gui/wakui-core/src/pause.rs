//! Composable pause ownership: releasing one owner never releases another.
//!
//! Toolkit-free gate shared by every native backend. Owners compose as
//! bits; the session pauses while any owner holds. An explicit user pause
//! is adopted only when acquiring the first hold, so a settings or modal
//! hold never manifests as a user hold on release.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PauseOwner {
    User,
    Focus,
    Modal,
    Settings,
}

#[derive(Debug, Default)]
pub struct PausePolicy {
    owners: u8,
}

impl PausePolicy {
    pub fn paused(&self) -> bool {
        self.owners != 0
    }

    pub fn owns(&self, owner: PauseOwner) -> bool {
        self.owners & (1 << owner as u8) != 0
    }

    /// Adopt an explicit pause only when acquiring the first hold.
    pub fn set(&mut self, owner: PauseOwner, active: bool, observed_paused: bool) -> bool {
        if active && !self.paused() && observed_paused {
            self.owners |= 1 << PauseOwner::User as u8;
        }
        let bit = 1 << owner as u8;
        if active {
            self.owners |= bit;
        } else {
            self.owners &= !bit;
        }
        self.paused()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_release_order_preserves_other_owners() {
        for first in [PauseOwner::Focus, PauseOwner::Modal, PauseOwner::Settings] {
            for second in [PauseOwner::Focus, PauseOwner::Modal, PauseOwner::Settings] {
                if first as u8 == second as u8 {
                    continue;
                }
                let mut policy = PausePolicy::default();
                assert!(policy.set(first, true, false));
                assert!(policy.set(second, true, true));
                assert!(policy.set(first, false, true));
                assert!(!policy.set(second, false, true));
            }
        }
    }

    #[test]
    fn explicit_pause_and_resume_under_hold_preserve_ownership() {
        let mut policy = PausePolicy::default();
        policy.set(PauseOwner::Modal, true, true);
        policy.set(PauseOwner::Focus, true, true);
        assert!(policy.set(PauseOwner::Modal, false, true));
        assert!(policy.set(PauseOwner::User, false, true));
        assert!(!policy.set(PauseOwner::Focus, false, true));
        policy.set(PauseOwner::User, true, false);
        policy.set(PauseOwner::Modal, true, true);
        assert!(policy.set(PauseOwner::Modal, false, true));
        assert!(policy.owns(PauseOwner::User));
    }
}
