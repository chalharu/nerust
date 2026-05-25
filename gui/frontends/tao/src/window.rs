// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod runtime;

use nerust_gui_shell::load::{NesLoadOptions, NesMmc3IrqVariant as ShellMmc3IrqVariant};
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

fn nes_load_options_from_window_options(options: WindowLoadOptions) -> NesLoadOptions {
    NesLoadOptions {
        mmc3_irq_variant: options.mmc3_irq_variant.map(shell_mmc3_irq_variant),
    }
}

fn shell_mmc3_irq_variant(variant: WindowMmc3IrqVariant) -> ShellMmc3IrqVariant {
    match variant {
        WindowMmc3IrqVariant::Sharp => ShellMmc3IrqVariant::Sharp,
        WindowMmc3IrqVariant::Nec => ShellMmc3IrqVariant::Nec,
    }
}

pub struct Window {
    runtime: Box<WindowRuntime>,
}

impl Window {
    pub fn new() -> Self {
        Self {
            runtime: Box::new(WindowRuntime::new(NesLoadOptions::default())),
        }
    }

    pub fn with_load_options(options: WindowLoadOptions) -> Self {
        Self {
            runtime: Box::new(WindowRuntime::new(nes_load_options_from_window_options(
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
            nes_load_options_from_window_options(options),
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
    use super::{WindowLoadOptions, WindowMmc3IrqVariant, nes_load_options_from_window_options};
    use nerust_gui_shell::load::{NesLoadOptions, NesMmc3IrqVariant};

    #[test]
    fn window_load_options_translate_to_shell_load_options() {
        assert_eq!(
            nes_load_options_from_window_options(WindowLoadOptions {
                mmc3_irq_variant: Some(WindowMmc3IrqVariant::Sharp),
            }),
            NesLoadOptions {
                mmc3_irq_variant: Some(NesMmc3IrqVariant::Sharp),
            }
        );
    }
}
