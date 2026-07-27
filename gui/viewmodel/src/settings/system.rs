use std::sync::Arc;

use nerust_core_traits::{
    factory::{
        CoreFactory,
        descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsFieldKind},
    },
    identity::SystemId,
};
use nerust_settings_core::factory::{apply_settings_choice, resolve_label, settings_view};

use super::{
    EditorState,
    dto::{ChoiceView, SystemFieldView, SystemTabView},
    editor::{SettingsEditor, ViewModelError},
    property::ReadOnlyObservableProperty,
};

/// Per-system settings view model.
#[derive(Clone)]
pub struct SystemSettingsViewModel {
    editor: SettingsEditor,
    factory_id: Box<dyn SystemId>,
    display_name: &'static str,
    pub view: ReadOnlyObservableProperty<SystemTabView>,
}

impl SystemSettingsViewModel {
    pub fn new(editor: &SettingsEditor, factory: &Arc<dyn CoreFactory>) -> Self {
        let factory_id = factory.system_id();
        let display_name = factory.display_name();
        let current = editor.current();
        let initial = project_view(&current, factory.as_ref());
        drop(current);
        let view = editor
            .projections()
            .register(factory.display_name(), initial, {
                let factory = Arc::clone(factory);
                move |state| project_view(state, factory.as_ref())
            });
        Self {
            editor: editor.clone(),
            factory_id,
            display_name,
            view,
        }
    }

    pub fn system_id(&self) -> &dyn SystemId {
        self.factory_id.as_ref()
    }

    pub fn display_name(&self) -> &'static str {
        self.display_name
    }

    pub fn set_choice(
        &self,
        field: &SystemSettingsFieldId,
        choice: &SystemSettingsChoiceId,
    ) -> Result<(), ViewModelError> {
        let factory_id = self.factory_id.clone_box();
        let field = field.clone();
        let choice = choice.clone();
        self.editor.transact(move |state| {
            // Find factory by ID from the registry
            let factory = state
                .catalog
                .find_by_id(factory_id.as_ref())
                .cloned()
                .ok_or(ViewModelError::UnknownSystem(factory_id.to_string()))?;
            apply_settings_choice(factory.as_ref(), state.draft_mut(), &field, &choice)
                .map_err(|_| ViewModelError::InvalidSystemChoice)
        })
    }
}

fn project_view(state: &EditorState, factory: &dyn CoreFactory) -> SystemTabView {
    let system_id = factory.system_id();
    let view = settings_view(&state.draft, system_id.as_ref());
    let model = factory.settings_page(&view);
    let language = state.draft.shared.general.language;
    SystemTabView {
        system_id: system_id.clone_box(),
        label: factory.display_name().to_string(),
        fields: model
            .fields
            .iter()
            .map(|field| {
                let SystemSettingsFieldKind::Choice { selected, options } = &field.kind;
                let choices: Vec<ChoiceView<SystemSettingsChoiceId>> = options
                    .iter()
                    .map(|opt| ChoiceView {
                        value: opt.id.clone(),
                        label: resolve_label(opt.label_id, language, factory),
                    })
                    .collect();
                SystemFieldView {
                    id: field.id.clone(),
                    label: resolve_label(field.label_id, language, factory),
                    selected: selected.clone(),
                    choices,
                }
            })
            .collect(),
    }
}
