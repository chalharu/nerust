//! Two-phase debugger writes, shared by the shell and presentation adapters.
use crate::debugger::PendingWrite;
use nerust_core_traits::debugger::SpaceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WriteTransaction {
    #[default]
    Empty,
    Prepared {
        space: SpaceId,
        addr: u32,
        old: Option<u8>,
    },
    Staged {
        space: SpaceId,
        addr: u32,
        old: Option<u8>,
        value: u64,
    },
}

impl WriteTransaction {
    pub fn prepare(&mut self, space: SpaceId, addr: u32, old: Option<u8>) {
        *self = Self::Prepared { space, addr, old };
    }

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

    pub fn take_commit(&mut self) -> Option<(SpaceId, u32, u64)> {
        if let Self::Staged {
            space, addr, value, ..
        } = *self
        {
            *self = Self::Empty;
            Some((space, addr, value))
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        *self = Self::Empty;
    }
    pub fn on_state_changing_traffic(&mut self) {
        self.clear();
    }

    pub fn pending(&self) -> Option<PendingWrite> {
        match *self {
            Self::Staged {
                addr, old, value, ..
            } => Some(PendingWrite { addr, old, value }),
            _ => None,
        }
    }
}
