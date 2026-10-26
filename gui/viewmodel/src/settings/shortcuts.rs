use nerust_settings_core::{
    bindings::descriptors::{ShortcutDescriptor, shortcut_descriptors},
    editor::CaptureTarget,
    i18n::{UiText, text as ui_text},
    model::{language::AppLanguage, snapshot::SettingsSnapshot},
};

use super::{
    EditorState,
    dto::{BindingRowView, BindingSectionView},
    editor::SettingsEditor,
    input::binding_value,
    property::ReadOnlyObservableProperty,
};

/// Sub-view model for the global shortcuts section. Projected once per
/// editor state, independent of registered systems: shortcuts are global
/// so every system would project identical rows.
#[derive(Clone)]
pub struct ShortcutsViewModel {
    pub view: ReadOnlyObservableProperty<BindingSectionView>,
}

impl ShortcutsViewModel {
    pub fn new(editor: &SettingsEditor) -> Self {
        let current = editor.current();
        let initial = project_view(&current);
        drop(current);
        let view = editor
            .projections()
            .register("shortcuts", initial, project_view);
        Self { view }
    }
}

fn project_view(state: &EditorState) -> BindingSectionView {
    let language = state.draft.shared.general.language;
    let shortcut_rows: Vec<BindingRowView> = shortcut_descriptors()
        .iter()
        .map(|desc| project_shortcut_row(&state.draft, desc, language, &state.capture_target))
        .collect();
    BindingSectionView {
        label: ui_text(language, UiText::Shortcuts).to_string(),
        rows: shortcut_rows,
    }
}

fn project_shortcut_row(
    draft: &SettingsSnapshot,
    desc: &ShortcutDescriptor,
    language: AppLanguage,
    capture_target: &Option<CaptureTarget>,
) -> BindingRowView {
    let target = CaptureTarget::Shortcut(desc.action);
    let value = binding_value(draft, &target, capture_target, language);
    BindingRowView {
        target,
        label: desc.label.to_string(),
        value,
    }
}

#[cfg(test)]
mod tests {
    use nerust_settings_core::editor::CaptureTarget;

    use crate::settings::test_support::test_vm;

    #[test]
    fn shortcuts_lists_all_global_actions_once() {
        let vm = test_vm();
        let view = vm.shortcuts.view.get();
        // All 8 global actions stay reachable through the dedicated view.
        assert_eq!(view.rows.len(), 8);
        assert!(
            view.rows
                .iter()
                .all(|row| matches!(row.target, CaptureTarget::Shortcut(_))),
            "shortcuts view must hold only global shortcut targets"
        );
    }
}
