use super::ConsoleRunner;
use crate::{ConsoleError, ConsoleReply, state};
use nerust_contract_persistence::CanonicalMediaIdentity;
use nerust_core::Core;

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

    pub(super) fn canonical_media_identity_reply(
        &self,
        core: Option<&Core>,
    ) -> Result<ConsoleReply, ConsoleError> {
        core.ok_or_else(Self::core_not_loaded).map(|core| {
            ConsoleReply::CanonicalMediaIdentity(CanonicalMediaIdentity::rom(core.rom_identity()))
        })
    }

    pub(super) fn export_state_reply(
        &self,
        core: Option<&Core>,
    ) -> Result<ConsoleReply, ConsoleError> {
        core.ok_or_else(Self::core_not_loaded).and_then(|core| {
            let controller_state = self
                .controller
                .current_controller_state()
                .map_err(ConsoleError::Core)?;
            state::build_state_export(
                core,
                &self.screen,
                controller_state,
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
                self.controller.as_mut(),
                &mut self.frame_counter,
                &mut self.paused,
                bytes,
            )?;
            Ok(ConsoleReply::Unit)
        })
    }
}
