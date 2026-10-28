use std::path::Path;

use nerust_frontend_host::bootstrap::Boot;

use crate::runtime::WindowRuntime;

pub struct Window {
    runtime: Box<WindowRuntime>,
}

impl Window {
    pub fn new(boot: Boot) -> Self {
        Self {
            runtime: Box::new(WindowRuntime::new(boot)),
        }
    }

    pub fn load_path(&mut self, path: &Path) -> bool {
        self.runtime.load_path(path)
    }

    pub fn run(self) {
        let runtime = self.runtime;
        (*runtime).run();
    }
}
