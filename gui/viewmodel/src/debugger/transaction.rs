//! Two-phase memory write transaction: `Empty` → `Prepared`
//! (row selected, old byte read) → `Staged` (value parsed) →
//! commit consumes. Pure transitions; the session owns one
//! instance and applies the stale-kill rule around drains.

use nerust_core_traits::debugger::SpaceId;

use super::PendingWrite;

/// Two-phase memory write transaction: `Empty` → `Prepared` (row
/// selected, old byte read) → `Staged` (value parsed) → commit
/// consumes. Pure transitions; the session owns one instance and
/// applies the stale-kill rule around drains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WriteTransaction {
    /// No pending write.
    #[default]
    Empty,
    /// Row selected; value not yet staged.
    Prepared {
        space: SpaceId,
        addr: u32,
        old: Option<u8>,
    },
    /// Fully staged; the confirm row renders [`PendingWrite`].
    Staged {
        space: SpaceId,
        addr: u32,
        old: Option<u8>,
        value: u64,
    },
}

impl WriteTransaction {
    /// Fresh selection overwrites any state.
    pub fn prepare(&mut self, space: SpaceId, addr: u32, old: Option<u8>) {
        *self = Self::Prepared { space, addr, old };
    }

    /// Stage the parsed value; only `Prepared` accepts it.
    pub fn stage(&mut self, value: u64) {
        if let Self::Prepared { space, addr, old } = *self {
            *self = Self::Staged {
                space,
                addr,
                old,
                value,
            };
        }
    }

    /// Commit the staged write, consuming the transaction. `None`
    /// unless fully staged.
    pub fn take_commit(&mut self) -> Option<(SpaceId, u32, u64)> {
        match *self {
            Self::Staged {
                space, addr, value, ..
            } => {
                *self = Self::Empty;
                Some((space, addr, value))
            }
            _ => None,
        }
    }

    /// Drop any uncommitted transaction (cancel, space switch).
    pub fn clear(&mut self) {
        *self = Self::Empty;
    }

    /// State-changing traffic (step, refresh, pause switches) kills
    /// staged-but-uncommitted values from previous drains, whose old
    /// byte may no longer match. Select/stage/commit traffic never
    /// triggers this: it builds the transaction this drain consumes.
    pub fn on_state_changing_traffic(&mut self) {
        if !matches!(self, Self::Empty) {
            *self = Self::Empty;
        }
    }

    /// Staged write, if fully staged. The frontend renders the
    /// confirm row from this (no language here: presentation owns it).
    pub fn pending(&self) -> Option<PendingWrite> {
        match *self {
            Self::Staged {
                addr, old, value, ..
            } => Some(PendingWrite { addr, old, value }),
            _ => None,
        }
    }
}
