use nerust_contract_input::InputTopologyDescriptor;
use std::borrow::Cow;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemDescriptor {
    pub input_topology: InputTopologyDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SystemSettingsFieldId(pub Cow<'static, str>);

impl SystemSettingsFieldId {
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SystemSettingsChoiceId(pub Cow<'static, str>);

impl SystemSettingsChoiceId {
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemSettingsPageModel {
    pub fields: Arc<[SystemSettingsFieldModel]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemSettingsFieldModel {
    pub id: SystemSettingsFieldId,
    pub label: String,
    pub kind: SystemSettingsFieldKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemSettingsFieldKind {
    Choice {
        selected: SystemSettingsChoiceId,
        options: Arc<[SystemSettingsChoiceOption]>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemSettingsChoiceOption {
    pub id: SystemSettingsChoiceId,
    pub label: String,
}
