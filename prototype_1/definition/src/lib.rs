//! Extensible, host-only DTOs and their backend-independent lowering.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Common {
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    OneWay,
    TwoWay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub path: String,
    #[serde(default)]
    pub mode: Mode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Property<T> {
    Literal(T),
    Bind(Binding),
    Resource(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRef {
    pub path: String,
}

#[typetag::serde(tag = "type")]
pub trait Control: std::fmt::Debug + Send + Sync {
    fn lower(&self) -> Result<Node>;
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub common: Common,
    pub title: Property<String>,
    pub content: Box<dyn Control>,
    #[serde(default)]
    pub menubar: MenuBar,
}

#[typetag::serde(name = "wakui.Window")]
impl Control for Window {
    fn lower(&self) -> Result<Node> {
        Ok(Node::Window {
            id: self.common.id.clone(),
            title: self.title.clone(),
            content: Box::new(self.content.lower()?),
            menu: self
                .menubar
                .items
                .iter()
                .map(|item| item.lower())
                .collect::<Result<_>>()?,
        })
    }
}

fn default_gap() -> u32 {
    10
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    pub common: Common,
    #[serde(default = "default_gap")]
    pub gap: u32,
    pub children: Vec<Box<dyn Control>>,
}

#[typetag::serde(name = "wakui.Panel")]
impl Control for Panel {
    fn lower(&self) -> Result<Node> {
        Ok(Node::Panel {
            id: self.common.id.clone(),
            gap: self.gap,
            children: self
                .children
                .iter()
                .map(|child| child.lower())
                .collect::<Result<_>>()?,
        })
    }
}

macro_rules! text_control {
    ($name:ident, $tag:literal, $variant:ident) => {
        #[derive(Debug, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub common: Common,
            pub text: Property<String>,
        }
        #[typetag::serde(name = $tag)]
        impl Control for $name {
            fn lower(&self) -> Result<Node> {
                Ok(Node::$variant {
                    id: self.common.id.clone(),
                    text: self.text.clone(),
                })
            }
        }
    };
}
text_control!(Label, "wakui.Label", Label);
text_control!(TextBox, "wakui.TextBox", Entry);

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Button {
    pub common: Common,
    pub text: Property<String>,
    pub command: CommandRef,
}
#[typetag::serde(name = "wakui.Button")]
impl Control for Button {
    fn lower(&self) -> Result<Node> {
        Ok(Node::Button {
            id: self.common.id.clone(),
            text: self.text.clone(),
            command: self.command.clone(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemsControl {
    pub common: Common,
    pub items: Binding,
    pub item_key: String,
    pub item_template: String,
}
#[typetag::serde(name = "wakui.ItemsControl")]
impl Control for ItemsControl {
    fn lower(&self) -> Result<Node> {
        Ok(Node::Items {
            id: self.common.id.clone(),
            items: self.items.clone(),
            item_key: self.item_key.clone(),
            template: self.item_template.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub enum Node {
    Window {
        id: String,
        title: Property<String>,
        content: Box<Node>,
        menu: Vec<MenuNodeDef>,
    },
    Panel {
        id: String,
        gap: u32,
        children: Vec<Node>,
    },
    Label {
        id: String,
        text: Property<String>,
    },
    Entry {
        id: String,
        text: Property<String>,
    },
    Button {
        id: String,
        text: Property<String>,
        command: CommandRef,
    },
    Items {
        id: String,
        items: Binding,
        item_key: String,
        template: String,
    },
}

#[typetag::serde(tag = "type")]
pub trait MenuNode: std::fmt::Debug + Send + Sync {
    fn lower(&self) -> Result<MenuNodeDef>;
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuBar {
    pub items: Vec<Box<dyn MenuNode>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submenu {
    pub id: String,
    pub text: String,
    pub items: Vec<Box<dyn MenuNode>>,
}

#[typetag::serde(name = "wakui.Submenu")]
impl MenuNode for Submenu {
    fn lower(&self) -> Result<MenuNodeDef> {
        Ok(MenuNodeDef::Submenu {
            id: self.id.clone(),
            text: self.text.clone(),
            items: self
                .items
                .iter()
                .map(|item| item.lower())
                .collect::<Result<_>>()?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuItem {
    pub id: String,
    pub text: Property<String>,
    pub command: CommandRef,
}

#[typetag::serde(name = "wakui.MenuItem")]
impl MenuNode for MenuItem {
    fn lower(&self) -> Result<MenuNodeDef> {
        Ok(MenuNodeDef::Item {
            id: self.id.clone(),
            text: self.text.clone(),
            command: self.command.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub enum MenuNodeDef {
    Submenu {
        id: String,
        text: String,
        items: Vec<Self>,
    },
    Item {
        id: String,
        text: Property<String>,
        command: CommandRef,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataTemplate {
    pub data_type: String,
    pub root: Box<dyn Control>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema_version: u32,
    pub view_model: String,
    #[serde(default)]
    pub resources: BTreeMap<String, String>,
    #[serde(default)]
    pub templates: BTreeMap<String, DataTemplate>,
    pub root: Box<dyn Control>,
}

/// Explicit descriptor registry; typetag's private inventory is not reflected.
#[derive(Default)]
pub struct Registry {
    names: BTreeSet<&'static str>,
}
impl Registry {
    pub fn register(&mut self, name: &'static str) -> Result<()> {
        if !self.names.insert(name) {
            return Err(Error(format!("duplicate Control/Menu tag: {name}")));
        }
        Ok(())
    }
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }
}

pub fn register(registry: &mut Registry) -> Result<()> {
    for name in [
        "wakui.Window",
        "wakui.Panel",
        "wakui.Label",
        "wakui.TextBox",
        "wakui.Button",
        "wakui.ItemsControl",
        "wakui.Submenu",
        "wakui.MenuItem",
    ] {
        registry.register(name)?;
    }
    Ok(())
}
