//! Shared by HOST build.rs and TARGET demo. Never imports runtime or toolkit.
use wakui_contract::{
    CommandDescriptor, CommandKey, Contract, Descriptor, ListItem, PropertyDescriptor, PropertyKey,
    ValueType,
};

pub struct Settings;
#[derive(Clone, Debug)]
pub struct State {
    pub name: String,
    pub status: String,
    pub busy: bool,
    pub saved: u32,
    pub rows: Vec<ListItem>,
}
pub enum Input {
    Name(String),
    Save,
    Add,
    Reverse,
    Clear,
}

// One declaration produces the typed key and its host metadata.
macro_rules! property {
    ($key:ident, $metadata:ident, $name:literal, $ty:ty, $kind:expr, $read:expr, $write:expr) => {
        pub static $key: PropertyKey<Settings, $ty> = PropertyKey {
            name: $name,
            read: $read,
            write: $write,
        };
        fn $metadata() -> PropertyDescriptor {
            PropertyDescriptor {
                name: $name,
                rust_key: concat!(module_path!(), "::", stringify!($key)),
                value_type: $kind,
                writable: $key.write.is_some(),
            }
        }
    };
}
property!(
    NAME,
    name_descriptor,
    "name",
    String,
    ValueType::Text,
    |s| s.name.clone(),
    Some(Input::Name)
);
property!(
    STATUS,
    status_descriptor,
    "status",
    String,
    ValueType::Text,
    |s| s.status.clone(),
    None
);
property!(
    BUSY,
    busy_descriptor,
    "busy",
    bool,
    ValueType::Bool,
    |s| s.busy,
    None
);
property!(
    ROWS,
    rows_descriptor,
    "rows",
    Vec<ListItem>,
    ValueType::Items {
        contract: "sample.Row",
        key: "id",
        text: "name"
    },
    |s| s.rows.clone(),
    None
);

macro_rules! command {
    ($key:ident, $metadata:ident, $name:literal, $input:expr, $enabled:expr) => {
        pub static $key: CommandKey<Settings> = CommandKey {
            name: $name,
            input: || $input,
            enabled: $enabled,
        };
        fn $metadata() -> CommandDescriptor {
            CommandDescriptor {
                name: $name,
                rust_key: concat!(module_path!(), "::", stringify!($key)),
            }
        }
    };
}
command!(SAVE, save_descriptor, "save", Input::Save, |s| !s.busy
    && !s.name.trim().is_empty());
command!(ADD, add_descriptor, "add", Input::Add, |s| !s
    .name
    .trim()
    .is_empty()
    && s.rows.len() < 100);
command!(
    REVERSE,
    reverse_descriptor,
    "reverse",
    Input::Reverse,
    |s| s.rows.len() > 1
);
command!(CLEAR, clear_descriptor, "clear", Input::Clear, |s| !s
    .rows
    .is_empty());

impl Contract for Settings {
    type State = State;
    type Input = Input;
    fn descriptor() -> Descriptor {
        Descriptor {
            name: "sample.Settings",
            properties: vec![
                name_descriptor(),
                status_descriptor(),
                busy_descriptor(),
                rows_descriptor(),
            ],
            commands: vec![
                save_descriptor(),
                add_descriptor(),
                reverse_descriptor(),
                clear_descriptor(),
            ],
        }
    }
}
