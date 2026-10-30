//! An extension unknown to wakui itself. Linked only into the build process.
use serde::{Deserialize, Serialize};
use wakui_definition::{Common, Control, Node, Property, Registry, Result};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Heading {
    pub common: Common,
    pub text: Property<String>,
}

#[typetag::serde(name = "sample.Heading")]
impl Control for Heading {
    fn lower(&self) -> Result<Node> {
        Ok(Node::Label {
            id: self.common.id.clone(),
            text: self.text.clone(),
        })
    }
}

pub fn register(registry: &mut Registry) -> Result<()> {
    // Explicit anchor plus descriptor registration in the HOST process.
    let _ = std::any::TypeId::of::<Heading>();
    registry.register("sample.Heading")
}
