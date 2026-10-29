//! View-side suspension ownership: releasing one owner never releases another.
//!
//! This policy belongs to the application layer, not the UI framework: it
//! arbitrates content suspension across unrelated reasons (explicit user
//! request, focus loss, modal dialogs). The framework only transports the
//! underlying notifications (focus events, modal effects, commands); this
//! module decides what they mean for suspension.
//!
//! Owners compose as bits; content stays suspended while any owner holds.
//! The owner table ([`super::session::pause_owners`]) gives each hold its
//! meaning, plus the adoption policy (an explicit user hold adopted when the
//! first hold arrives over already suspended content).

/// Opaque suspension owner. Minted from the view-side owner table;
/// never a position or a framework-known kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PauseOwner(u8);

impl PauseOwner {
    /// Mint an owner token for the view-side owner table (index < 32).
    pub const fn new(index: u8) -> Self {
        Self(index)
    }

    /// Raw index for the view-side owner table. Diagnostics only.
    pub const fn index(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Default)]
pub struct PausePolicy {
    owners: u32,
}

impl PausePolicy {
    pub fn paused(&self) -> bool {
        self.owners != 0
    }

    pub fn owns(&self, owner: PauseOwner) -> bool {
        self.owners & (1 << owner.0) != 0
    }

    /// Set or release one opaque hold. Returns whether content stays suspended.
    /// Adoption policies (e.g. adopting an explicit user hold on the first
    /// acquisition) live next to the owner table.
    pub fn set(&mut self, owner: PauseOwner, active: bool) -> bool {
        debug_assert!(owner.0 < 32, "pause owner index exhausted");
        let bit = 1 << owner.0;
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

    const FIRST: PauseOwner = PauseOwner::new(1);
    const SECOND: PauseOwner = PauseOwner::new(2);
    const THIRD: PauseOwner = PauseOwner::new(3);

    #[test]
    fn every_release_order_preserves_other_owners() {
        for first in [FIRST, SECOND, THIRD] {
            for second in [FIRST, SECOND, THIRD] {
                if first == second {
                    continue;
                }
                let mut policy = PausePolicy::default();
                assert!(policy.set(first, true));
                assert!(policy.set(second, true));
                assert!(policy.set(first, false));
                assert!(!policy.set(second, false));
            }
        }
    }

    #[test]
    fn holds_compose_and_report_ownership() {
        let mut policy = PausePolicy::default();
        assert!(!policy.paused());
        assert!(policy.set(FIRST, true));
        assert!(policy.set(SECOND, true));
        assert!(policy.owns(FIRST));
        assert!(policy.set(FIRST, false));
        assert!(policy.owns(SECOND));
        assert!(!policy.set(SECOND, false));
        assert!(!policy.owns(FIRST));
    }
}
