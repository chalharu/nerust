// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod runtime;

use nerust_contract::CoreOptions;
use runtime::WindowRuntime;
use std::path::PathBuf;

pub struct Window {
    runtime: Box<WindowRuntime>,
}

impl Window {
    pub fn new() -> Self {
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
        options: CoreOptions,
    ) {
        self.runtime.load_with_options(rom_path, data, options);
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
