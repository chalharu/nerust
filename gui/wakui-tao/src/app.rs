use std::path::Path;

use wakui::bootstrap::Application;

use crate::runtime::WindowRuntime;

pub struct Window<K: Copy + Eq + std::hash::Hash + 'static> {
    runtime: Box<WindowRuntime<K>>,
}

impl<K: Copy + Eq + std::hash::Hash + 'static> Window<K> {
    pub fn new(app: Application<K>) -> Self {
        Self {
            runtime: Box::new(WindowRuntime::new(app)),
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
