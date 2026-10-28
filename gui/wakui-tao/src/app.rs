use std::path::Path;

use wakui::bootstrap::Boot;

use crate::runtime::WindowRuntime;

pub struct Window<K: Copy + Eq + std::hash::Hash + 'static> {
    runtime: Box<WindowRuntime<K>>,
}

impl<K: Copy + Eq + std::hash::Hash + 'static> Window<K> {
    pub fn new(boot: Boot<K>) -> Self {
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
