use super::GuiSession;
use nerust_persistence::sidecar::{
    load_mapper_save, write_mapper_save, write_recovery_mapper_save,
};

impl GuiSession {
    pub(super) fn load_mapper_save_if_available(&mut self) -> Result<(), String> {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return Ok(());
        };
        if let Some(bytes) =
            load_mapper_save(&sidecars.mapper_save_path).map_err(|error| error.to_string())?
        {
            self.core
                .import_mapper_save(bytes)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub(super) fn flush_mapper_save(&mut self) -> Result<(), String> {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return Ok(());
        };
        if !self.persistence.mapper_save_flush_allowed {
            if self.persistence.mapper_save_recovery_written {
                return Ok(());
            }
            if let Some(bytes) = self
                .core
                .export_mapper_save()
                .map_err(|error| error.to_string())?
            {
                let path = write_recovery_mapper_save(&sidecars.mapper_save_path, &bytes)
                    .map_err(|error| error.to_string())?;
                self.persistence.mapper_save_recovery_written = true;
                log::warn!(
                    "mapper save auto-load failed earlier; wrote recovery save to {}",
                    path.display()
                );
            }
            return Ok(());
        }
        match self
            .core
            .export_mapper_save()
            .map_err(|error| error.to_string())?
        {
            Some(bytes) => write_mapper_save(&sidecars.mapper_save_path, &bytes)
                .map_err(|error| error.to_string()),
            None => Ok(()),
        }
    }
}
