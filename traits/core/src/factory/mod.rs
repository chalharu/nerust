pub mod descriptor;
pub mod load;

use crate::ConsoleCore;
use nerust_input_traits::SystemInputAdapter;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FactoryError {
    #[error("core creation failed: {0}")]
    Create(String),
    #[error("invalid settings choice: {0}")]
    InvalidChoice(String),
    #[error("load request resolution failed: {0}")]
    Resolve(String),
}

/// Raw parts produced by a system factory before EmuCore wrapping.
pub struct CoreParts {
    pub core: Box<dyn ConsoleCore>,
    pub adapter: Box<dyn SystemInputAdapter>,
    pub render_profile: nerust_render_base::VideoRenderProfile,
}
