use std::{cell::RefCell, path::PathBuf, rc::Rc};
use wakui2_macros::commands;
use wakui2_runtime::{BlockingPool, Executor, Keyed, MenuItem, MenuRole, Policy, Store, Viewport};

#[derive(Clone)]
pub struct Row {
    pub name: String,
    pub progress: f64,
}
#[commands]
impl Row {
    #[command]
    fn rename(&mut self, argument: String) {
        self.name = argument.to_uppercase();
    }
}

#[derive(Clone)]
pub struct Settings {
    pub name: String,
    pub name_error: String,
    pub validating: bool,
    pub progress: f64,
    pub status: String,
    pub busy: bool,
    pub saved: usize,
    pub rows: Vec<Keyed<Row>>,
    pub selected: Option<Keyed<Row>>,
    pub viewport: Viewport,
    pub menu_items: Vec<MenuItem>,
    workers: Rc<RefCell<BlockingPool>>,
    path: PathBuf,
}
impl Settings {
    pub fn new(
        executor: Rc<dyn Executor>,
        workers: Rc<RefCell<BlockingPool>>,
        path: PathBuf,
    ) -> Self {
        let rows: Vec<_> = (0..1000)
            .map(|i| {
                Keyed::new(
                    i.to_string(),
                    Store::new(
                        Row {
                            name: format!("Item {i}"),
                            progress: (i % 10) as f64 / 10.0,
                        },
                        executor.clone(),
                    ),
                )
            })
            .collect();
        Self {
            name: "Wakui 2".into(),
            name_error: String::new(),
            validating: false,
            progress: 0.5,
            status: "Ready — 1000 item models, virtualized controls".into(),
            busy: false,
            saved: 0,
            selected: Some(rows[0].clone()),
            rows,
            viewport: Viewport {
                start: 0,
                count: 4,
                row_height: 64.0,
            },
            menu_items: vec![
                menu_item("save", "Save", "save", Some(shortcut())),
                menu_item("reverse", "Reverse items", "reverse", None),
                MenuItem {
                    id: "separator".into(),
                    label: String::new(),
                    argument: String::new(),
                    shortcut: None,
                    role: MenuRole::Separator,
                    enabled: true,
                },
                MenuItem {
                    id: "about".into(),
                    label: "About Wakui".into(),
                    argument: String::new(),
                    shortcut: None,
                    role: MenuRole::About,
                    enabled: true,
                },
            ],
            workers,
            path,
        }
    }
    fn can_save(&self) -> bool {
        !self.busy && !self.validating && self.name_error.is_empty() && !self.name.trim().is_empty()
    }
}
fn shortcut() -> String {
    #[cfg(feature = "backend-gtk")]
    {
        "<Control>s".into()
    }
    #[cfg(all(feature = "backend-tao", not(feature = "backend-gtk")))]
    {
        "Ctrl+KeyS".into()
    }
}
fn menu_item(id: &str, label: &str, argument: &str, shortcut: Option<String>) -> MenuItem {
    MenuItem {
        id: id.into(),
        label: label.into(),
        argument: argument.into(),
        shortcut,
        role: MenuRole::Normal,
        enabled: true,
    }
}

#[commands]
impl Settings {
    #[command]
    fn reverse(&mut self) {
        self.rows.reverse();
        self.selected = self.rows.first().cloned();
        self.menu_items[1].label = format!("Reverse again (first: {})", self.rows[0].key);
    }
    #[command]
    fn choose(model: Store<Self>, argument: String) {
        match argument.as_str() {
            "save" => Self::save(model, String::new()),
            "reverse" => {
                let _ = model.update(|s| s.reverse());
            }
            _ => {
                let _ = model.update(move |s| s.status = format!("Menu argument: {argument}"));
            }
        }
    }
    #[command(can_execute = "can_save")]
    fn save(model: Store<Self>, _: String) {
        if !model.snapshot().can_save() {
            return;
        }
        let state = model.snapshot();
        let result = state
            .workers
            .borrow()
            .try_spawn(move || atomic_save(&state.path, state.name.as_bytes()));
        match result {
            Ok(task) => {
                let _ = model.update(|s| {
                    s.busy = true;
                    s.status = "Saving…".into();
                    s.menu_items[0].enabled = false;
                });
                if !model.effect(Policy::Serial, "save", task, |s, result| {
                    s.busy = false;
                    s.menu_items[0].enabled = true;
                    s.status = match result.map_err(|e| e.to_string()).and_then(|r| r) {
                        Ok(()) => {
                            s.saved += 1;
                            format!("Saved ({})", s.saved)
                        }
                        Err(error) => format!("Error: {error}"),
                    };
                }) {
                    let _ = model.update(|s| {
                        s.busy = false;
                        s.menu_items[0].enabled = true;
                        s.status = "Task capacity exceeded".into();
                    });
                }
            }
            Err(error) => {
                let _ = model.update(move |s| s.status = error.to_string());
            }
        }
    }
}

fn atomic_save(path: &std::path::Path, value: &[u8]) -> Result<(), String> {
    use std::{fs, io::Write};
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut created = false;
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        created = true;
        file.write_all(value)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(temp);
    }
    result.map_err(|error| error.to_string())
}
