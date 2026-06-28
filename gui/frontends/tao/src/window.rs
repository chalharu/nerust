mod runtime;

use std::path::Path;

use nerust_gui_shell::context::FrontendContext;
use runtime::WindowRuntime;

pub struct Window {
    runtime: Box<WindowRuntime>,
}

impl Window {
    pub fn new(ctx: FrontendContext) -> Self {
        Self {
            runtime: Box::new(WindowRuntime::new(ctx)),
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
