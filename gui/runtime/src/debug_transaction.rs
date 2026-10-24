//! Two-phase debugger writes, shared by the shell and presentation adapters.
use nerust_core_traits::debugger::SpaceId;
use nerust_settings_core::model::language::AppLanguage;

/// A staged write awaiting confirmation, independent of the debugger display cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PendingWrite {
    pub addr: u32,
    pub old: Option<u8>,
    pub value: u64,
}

impl PendingWrite {
    pub fn render(&self, language: AppLanguage) -> String {
        nerust_settings_core::i18n::pending_write(language, self.addr, self.old, self.value)
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_write_exposes_pending_model_and_commits_once() {
        let mut transaction = WriteTransaction::default();
        transaction.prepare(SpaceId(3), 0x1234, Some(0x56));
        assert_eq!(transaction.pending(), None);
        assert_eq!(transaction.take_commit(), None);

        transaction.stage(0xABCD);
        assert_eq!(
            transaction.pending(),
            Some(PendingWrite {
                addr: 0x1234,
                old: Some(0x56),
                value: 0xABCD,
            })
        );
        assert_eq!(
            transaction.take_commit(),
            Some((SpaceId(3), 0x1234, 0xABCD))
        );
        assert_eq!(transaction.pending(), None);
        assert_eq!(transaction.take_commit(), None);
        assert_eq!(transaction, WriteTransaction::Empty);
    }

    #[test]
    fn state_changing_traffic_discards_prepared_and_staged_writes() {
        for staged in [false, true] {
            let mut transaction = WriteTransaction::default();
            transaction.prepare(SpaceId(1), 0x20, None);
            if staged {
                transaction.stage(0x42);
            }
            transaction.on_state_changing_traffic();
            assert_eq!(transaction, WriteTransaction::Empty);
            assert_eq!(transaction.pending(), None);
            assert_eq!(transaction.take_commit(), None);
        }
    }

    #[test]
    fn pending_model_preserves_unknown_old_value_and_rendering() {
        let mut transaction = WriteTransaction::default();
        transaction.stage(0xFF);
        assert_eq!(transaction, WriteTransaction::Empty);
        transaction.prepare(SpaceId(0), 0x20, None);
        transaction.stage(0x42);
        let pending = transaction.pending().expect("staged write");
        assert_eq!(pending.old, None);
        for language in [AppLanguage::Japanese, AppLanguage::English] {
            assert_eq!(
                pending.render(language),
                nerust_settings_core::i18n::pending_write(language, 0x20, None, 0x42)
            );
        }
    }
}
