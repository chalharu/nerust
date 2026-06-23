mod runtime;

use nerust_gui_shell::load::{LoadRequest, SystemLoadOptions};
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
    let options_bytes = match options.mmc3_irq_variant {
        Some(WindowMmc3IrqVariant::Sharp) => nerust_factory_nes::MMC3_OPTION_SHARP.to_vec(),
        Some(WindowMmc3IrqVariant::Nec) => nerust_factory_nes::MMC3_OPTION_NEC.to_vec(),
        None => Vec::new(),
    };
    LoadRequest::Explicit {
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

    #[test]
    fn window_load_options_translate_to_system_load_request() {
        let request = system_load_request_from_window_options(WindowLoadOptions {
            mmc3_irq_variant: Some(WindowMmc3IrqVariant::Sharp),
        });
        let LoadRequest::Explicit { options } = request else {
            panic!("expected Explicit load request, got {request:?}");
        };
        assert!(!options.options_bytes.is_empty());
    }
}
