use super::ConsoleRunner;
use crate::{ConsoleError, ConsoleReply, PersistenceTarget, core_api::Core, state};

impl ConsoleRunner {
    pub(super) fn export_mapper_save_reply(
        &self,
        core: Option<&Core>,
    ) -> Result<ConsoleReply, ConsoleError> {
        core.ok_or_else(Self::core_not_loaded).and_then(|core| {
            core.export_mapper_save()
                .map(ConsoleReply::MapperSave)
                .map_err(|error| ConsoleError::Core(error.to_string()))
        })
    }

    pub(super) fn import_mapper_save_reply(
        &self,
        core: Option<&mut Core>,
        bytes: &[u8],
    ) -> Result<ConsoleReply, ConsoleError> {
        core.ok_or_else(Self::core_not_loaded).and_then(|core| {
            core.import_mapper_save(bytes)
                .map(|_| ConsoleReply::Unit)
                .map_err(|error| ConsoleError::Core(error.to_string()))
        })
    }

    pub(super) fn persistence_target_reply(
        &self,
        core: Option<&Core>,
    ) -> Result<ConsoleReply, ConsoleError> {
        core.ok_or_else(Self::core_not_loaded).map(|core| {
            ConsoleReply::PersistenceTarget(PersistenceTarget {
                rom_identity: core.rom_identity(),
                options: core.options(),
            })
        })
    }

    pub(super) fn export_state_reply(
        &self,
        core: Option<&Core>,
    ) -> Result<ConsoleReply, ConsoleError> {
        core.ok_or_else(Self::core_not_loaded).and_then(|core| {
            state::build_state_export(
                core,
                &self.screen,
                &self.controller,
                self.frame_counter,
                self.paused,
            )
            .map(ConsoleReply::StateExport)
        })
    }

    pub(super) fn import_state_reply(
        &mut self,
        core: Option<&mut Core>,
        bytes: &[u8],
    ) -> Result<ConsoleReply, ConsoleError> {
        core.ok_or_else(Self::core_not_loaded).and_then(|core| {
            state::restore_imported_state(
                core,
                &mut self.screen,
                &mut self.controller,
                &mut self.frame_counter,
                &mut self.paused,
                bytes,
            )?;
            Ok(ConsoleReply::Unit)
        })
    }
}
