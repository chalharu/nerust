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
    let options_bytes = options
        .mmc3_irq_variant
        .map(|v| {
            let core_opts = nerust_nes_core::core_options::CoreOptions {
                mmc3_irq_variant: Some(match v {
                    WindowMmc3IrqVariant::Sharp => {
                        nerust_nes_core::core_options::Mmc3IrqVariant::Sharp
                    }
                    WindowMmc3IrqVariant::Nec => nerust_nes_core::core_options::Mmc3IrqVariant::Nec,
                }),
            };
            core_opts.into_bytes()
        })
        .unwrap_or_default();
    LoadRequest::Explicit {
        system_id: SystemId::Nes,
        options: SystemLoadOptions { options_bytes },
    }
}

pub struct Window {
    runtime: Box<WindowRuntime>,
}

impl Window {
    pub fn new() -> Self {
        Self {
            runtime: Box::new(WindowRuntime::new()),
        }
    }

    pub fn with_load_options(options: WindowLoadOptions) -> Self {
        let _ = options;
        Self {
            runtime: Box::new(WindowRuntime::new()),
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
    use nerust_gui_shell::load::LoadRequest;
    use nerust_input_schema::SystemId;

    #[test]
    fn window_load_options_translate_to_system_load_request() {
        let request = system_load_request_from_window_options(WindowLoadOptions {
            mmc3_irq_variant: Some(WindowMmc3IrqVariant::Sharp),
        });
        let LoadRequest::Explicit { system_id, options } = request else {
            panic!("expected Explicit load request");
        };
        assert_eq!(system_id, SystemId::Nes);
        assert!(!options.options_bytes.is_empty());
    }
}
