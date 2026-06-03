mod runtime;

use nerust_gui_shell::load::{LoadRequest, SystemLoadOptions};
use nerust_input_schema::SystemId;
use runtime::WindowRuntime;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowLoadOptions {
    pub mmc3_irq_variant: Option<WindowMmc3IrqVariant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowMmc3IrqVariant {
    Sharp,
    Nec,
}

fn system_load_request_from_window_options(options: WindowLoadOptions) -> LoadRequest {
    if let Some(mmc3_irq_variant) = options.mmc3_irq_variant {
        return LoadRequest::Explicit {
            system_id: SystemId::Nes,
            options: SystemLoadOptions {
                mmc3_irq_variant: Some(shell_mmc3_irq_variant(mmc3_irq_variant)),
            },
        };
    }

    LoadRequest::Auto
}

fn shell_mmc3_irq_variant(
    variant: WindowMmc3IrqVariant,
) -> nerust_contract_options::Mmc3IrqVariant {
    match variant {
        WindowMmc3IrqVariant::Sharp => nerust_contract_options::Mmc3IrqVariant::Sharp,
        WindowMmc3IrqVariant::Nec => nerust_contract_options::Mmc3IrqVariant::Nec,
    }
}

pub struct Window {
    runtime: Box<WindowRuntime>,
}

impl Window {
    pub fn new() -> Self {
        Self {
            runtime: Box::new(WindowRuntime::new(LoadRequest::Auto)),
        }
    }

    pub fn with_load_options(options: WindowLoadOptions) -> Self {
        Self {
            runtime: Box::new(WindowRuntime::new(system_load_request_from_window_options(
                options,
            ))),
        }
    }

    pub fn load(&mut self, data: Vec<u8>) {
        self.runtime.load(data);
    }

    pub fn load_with_options(
        &mut self,
        rom_path: Option<PathBuf>,
        data: Vec<u8>,
        options: WindowLoadOptions,
    ) {
        self.runtime.load_with_options(
            rom_path,
            data,
            system_load_request_from_window_options(options),
        );
    }

    pub fn load_path(&mut self, path: &Path) -> bool {
        self.runtime.load_path(path)
    }

    pub fn run(self) {
        let runtime = self.runtime;
        (*runtime).run();
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{WindowLoadOptions, WindowMmc3IrqVariant, system_load_request_from_window_options};
    use nerust_contract_options::Mmc3IrqVariant;
    use nerust_gui_shell::load::{LoadRequest, SystemLoadOptions};
    use nerust_input_schema::SystemId;

    #[test]
    fn window_load_options_translate_to_system_load_request() {
        assert_eq!(
            system_load_request_from_window_options(WindowLoadOptions {
                mmc3_irq_variant: Some(WindowMmc3IrqVariant::Sharp),
            }),
            LoadRequest::Explicit {
                system_id: SystemId::Nes,
                options: SystemLoadOptions {
                    mmc3_irq_variant: Some(Mmc3IrqVariant::Sharp),
                },
            }
        );
    }

    #[test]
    fn default_window_load_options_keep_system_auto_detection() {
        assert_eq!(
            system_load_request_from_window_options(WindowLoadOptions::default()),
            LoadRequest::Auto
        );
    }
}
