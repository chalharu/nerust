use nerust_settings_core::{
    bindings::descriptors::{ShortcutDescriptor, shortcut_descriptors},
    editor::CaptureTarget,
    i18n::{I18n, UiText},
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
    let i18n = I18n::resolve_os(state.draft.shared.general.language);
    let shortcut_rows: Vec<BindingRowView> = shortcut_descriptors()
        .iter()
        .map(|desc| project_shortcut_row(state, desc, i18n))
        .collect();
    BindingSectionView {
        label: i18n.text(UiText::Shortcuts),
        rows: shortcut_rows,
    }
}

fn project_shortcut_row(
    state: &EditorState,
    desc: &ShortcutDescriptor,
    i18n: I18n,
) -> BindingRowView {
    let target = CaptureTarget::Shortcut(desc.action);
    let value = binding_value(state, &target, i18n);
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
    fn shortcut_row_combines_key_and_pad_labels() {
        use std::sync::Arc;

        use nerust_settings_core::model::{
            app_state::DesktopAppState,
            input::{ShortcutAction, default_shortcut_bindings},
            local::HostBackendLocalSettings,
            shared::DesktopSharedSettings,
            snapshot::SettingsSnapshot,
        };

        use super::super::dto::BindingValueView;
        use crate::settings::editor::NoopStoragePathValidator;
        use crate::settings::test_support::{TestCoreFactory, TestInputFactory};

        let mut shared = DesktopSharedSettings::default();
        shared.input.shortcuts.keyboard = default_shortcut_bindings();
        let snapshot = SettingsSnapshot {
            shared,
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        };
        let factory: Arc<dyn nerust_system_traits::factory::CoreFactory> =
            Arc::new(TestCoreFactory(TestInputFactory::new()));
        let vm = crate::settings::SettingsViewModel::new(
            snapshot,
            vec![factory],
            Arc::new([]),
            std::rc::Rc::new(NoopStoragePathValidator)
                as std::rc::Rc<dyn crate::settings::editor::StoragePathValidator>,
        )
        .unwrap();
        let rows = vm.shortcuts.view.get().rows;
        let pause = rows
            .iter()
            .find(|row| row.target == CaptureTarget::Shortcut(ShortcutAction::TogglePause))
            .expect("pause row");
        assert!(
            matches!(&pause.value, BindingValueView::Bound(label) if label == "Space + Guide"),
            "pause row shows both columns, got {:?}",
            pause.value
        );
        let reset = rows
            .iter()
            .find(|row| row.target == CaptureTarget::Shortcut(ShortcutAction::Reset))
            .expect("reset row");
        assert!(
            matches!(&reset.value, BindingValueView::Unbound(_)),
            "unbound row stays unbound, got {:?}",
            reset.value
        );
    }

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
