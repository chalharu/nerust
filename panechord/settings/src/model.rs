//! Pure, small snapshots. No View, executor, filesystem or worker pool in state.
use crate::persistence::Config;
use panechord_macros::commands;
use panechord_runtime::{MenuItem, MenuRole, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub name: String,
    pub volume: String,
    pub scale: String,
}
impl From<&Config> for Draft {
    fn from(config: &Config) -> Self {
        Self {
            name: config.name.clone(),
            volume: config.volume.to_string(),
            scale: config.scale.to_string(),
        }
    }
}
impl Draft {
    pub fn parse(&self) -> Result<Config, String> {
        let config = Config {
            schema_version: 1,
            name: self.name.clone(),
            volume: self
                .volume
                .parse()
                .map_err(|_| "Volume must be an integer from 0 to 100")?,
            scale: self
                .scale
                .parse()
                .map_err(|_| "Scale must be a finite number from 0.5 to 3")?,
        };
        config.validate()?;
        Ok(config)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveIntent {
    Stay,
    Exit,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Editing,
    Confirm,
    Waiting,
    Ready,
}
#[derive(Debug, Clone, PartialEq)]
pub struct SaveRequest {
    pub config: Config,
    pub draft: Draft,
}

#[derive(Clone, PartialEq, Default)]
pub struct Projection {
    pub error: String,
    pub dirty: bool,
    pub summary: String,
    pub warning: String,
    pub status: String,
    pub menu_items: Vec<MenuItem>,
}
#[derive(Clone)]
pub struct App {
    pub saved: Config,
    pub draft: Draft,
    pub pending: Option<SaveRequest>,
    pub exit: Exit,
    pub io_error: String,
    pub ui: Projection,
    pub stay: SaveIntent,
    pub leave: SaveIntent,
}
impl App {
    pub fn new(saved: Config) -> Self {
        let mut state = Self {
            draft: Draft::from(&saved),
            saved,
            pending: None,
            exit: Exit::Editing,
            io_error: String::new(),
            ui: Projection::default(),
            stay: SaveIntent::Stay,
            leave: SaveIntent::Exit,
        };
        state.ui = state.project();
        state
    }
    pub fn dirty(&self) -> bool {
        self.draft != Draft::from(&self.saved)
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn can_save(&self) -> bool {
        !self.busy() && self.exit != Exit::Ready && self.dirty() && self.draft.parse().is_ok()
    }
    fn can_save_exit(&self) -> bool {
        self.exit == Exit::Confirm && self.can_save()
    }
    fn can_cancel(&self) -> bool {
        !self.busy() && self.exit != Exit::Ready
    }
    fn can_discard(&self) -> bool {
        self.exit == Exit::Confirm && !self.busy()
    }
    fn can_keep(&self) -> bool {
        matches!(self.exit, Exit::Confirm | Exit::Waiting)
    }
    pub fn close_ready(&self) -> bool {
        self.exit == Exit::Ready
    }
    pub fn project(&self) -> Projection {
        let error = self.draft.parse().err().unwrap_or_default();
        Projection {
            error,
            dirty: self.dirty(),
            summary: format!(
                "Saved: {} | volume {} | scale {}",
                self.saved.name, self.saved.volume, self.saved.scale
            ),
            warning: match self.exit {
                Exit::Confirm => "Unsaved changes. Save and exit, discard, or keep editing.",
                Exit::Waiting => "Waiting for save. New edits will require confirmation again.",
                _ => "",
            }
            .into(),
            status: if !self.io_error.is_empty() {
                self.io_error.clone()
            } else if self.busy() {
                "Saving…".into()
            } else if self.dirty() {
                "Unsaved changes".into()
            } else {
                "Up to date".into()
            },
            menu_items: vec![
                MenuItem {
                    id: "save".into(),
                    label: "Save".into(),
                    argument: "save".into(),
                    shortcut: Some("Primary+S".into()),
                    role: MenuRole::Normal,
                    enabled: self.can_save(),
                },
                MenuItem {
                    id: "quit".into(),
                    label: "Quit".into(),
                    argument: String::new(),
                    shortcut: Some("Primary+Q".into()),
                    role: MenuRole::Quit,
                    enabled: true,
                },
            ],
        }
    }
    pub fn request_exit(model: Store<Self>) {
        let _ = model.update(|s| {
            s.exit = if s.busy() {
                Exit::Waiting
            } else if s.dirty() {
                Exit::Confirm
            } else {
                Exit::Ready
            };
        });
    }
    pub fn complete(&mut self, request: SaveRequest, result: Result<(), String>) {
        self.pending = None;
        match result {
            Ok(()) => {
                let unchanged = self.draft == request.draft
                    && self.draft.parse().as_ref() == Ok(&request.config);
                self.saved = request.config;
                self.io_error.clear();
                // Normalize only the captured draft, never a newer edit.
                if unchanged {
                    self.draft = Draft::from(&self.saved);
                }
                if self.exit == Exit::Waiting {
                    self.exit = if unchanged {
                        Exit::Ready
                    } else {
                        Exit::Confirm
                    };
                }
            }
            Err(error) => {
                self.io_error = error;
                if self.exit == Exit::Waiting {
                    self.exit = Exit::Confirm;
                }
            }
        }
    }
}
#[commands]
impl App {
    #[command(can_execute = "can_save")]
    fn save(&mut self, intent: SaveIntent) {
        if !self.can_save() {
            return;
        }
        if let Ok(config) = self.draft.parse() {
            self.pending = Some(SaveRequest {
                config,
                draft: self.draft.clone(),
            });
            self.io_error.clear();
            if intent == SaveIntent::Exit {
                self.exit = Exit::Waiting;
            }
        }
    }
    #[command(can_execute = "can_save_exit")]
    fn save_exit(&mut self, intent: SaveIntent) {
        self.save(intent);
    }
    #[command(can_execute = "can_cancel")]
    fn cancel(&mut self) {
        self.draft = Draft::from(&self.saved);
        self.io_error.clear();
        self.exit = Exit::Editing;
    }
    #[command(can_execute = "can_discard")]
    fn discard_exit(&mut self) {
        self.exit = Exit::Ready;
    }
    #[command(can_execute = "can_keep")]
    fn keep_editing(&mut self) {
        self.exit = Exit::Editing;
    }
    #[command]
    fn choose(model: Store<Self>, argument: String) {
        match argument.as_str() {
            "save" => {
                let _ = model
                    .command_save(panechord_runtime::Binding::constant(SaveIntent::Stay))
                    .invoke();
            }
            "quit" => Self::request_exit(model),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
