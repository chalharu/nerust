use super::{
    dto::NotificationView,
    editor::{SettingsEditor, ViewModelError},
    property::ReadOnlyObservableProperty,
};
use nerust_settings_core::model::local::{
    NotificationAreaMode, NotificationColor, NotificationPosition,
};

#[derive(Clone)]
pub struct NotificationSettingsViewModel {
    _editor: SettingsEditor,
    pub view: ReadOnlyObservableProperty<NotificationView>,
}

impl NotificationSettingsViewModel {
    pub fn new(editor: &SettingsEditor) -> Self {
        let current = editor.current();
        let initial = project_view(&current);
        drop(current);
        let view = editor
            .projections()
            .register("notifications", initial, project_view);
        Self {
            _editor: editor.clone(),
            view,
        }
    }

    pub fn set_info_enabled(&self, value: bool) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.notifications.info_enabled = value;
            Ok(())
        })
    }

    pub fn set_info_duration_ms(&self, value: u32) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.notifications.info_duration_ms = value;
            Ok(())
        })
    }

    pub fn set_info_color(&self, value: NotificationColor) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.notifications.info_color = value;
            Ok(())
        })
    }

    pub fn set_warning_enabled(&self, value: bool) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.notifications.warning_enabled = value;
            Ok(())
        })
    }

    pub fn set_warning_duration_ms(&self, value: u32) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.notifications.warning_duration_ms = value;
            Ok(())
        })
    }

    pub fn set_warning_color(&self, value: NotificationColor) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.notifications.warning_color = value;
            Ok(())
        })
    }

    pub fn set_error_enabled(&self, value: bool) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.notifications.error_enabled = value;
            Ok(())
        })
    }

    pub fn set_error_modal(&self, value: bool) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.notifications.error_modal = value;
            Ok(())
        })
    }

    pub fn set_error_color(&self, value: NotificationColor) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.notifications.error_color = value;
            Ok(())
        })
    }

    pub fn set_status_enabled(&self, value: bool) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.notifications.status_enabled = value;
            Ok(())
        })
    }

    pub fn set_position(&self, value: NotificationPosition) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.notifications.position = value;
            Ok(())
        })
    }

    pub fn set_status_position(&self, value: NotificationPosition) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.notifications.status_position = value;
            Ok(())
        })
    }

    pub fn set_area_mode(&self, value: NotificationAreaMode) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.notifications.area_mode = value;
            Ok(())
        })
    }
}

fn project_view(state: &super::EditorState) -> NotificationView {
    let n = &state.draft.local.notifications;
    NotificationView {
        info_enabled: n.info_enabled,
        info_duration_ms: n.info_duration_ms,
        info_color: n.info_color,
        warning_enabled: n.warning_enabled,
        warning_duration_ms: n.warning_duration_ms,
        warning_color: n.warning_color,
        error_enabled: n.error_enabled,
        error_modal: n.error_modal,
        error_color: n.error_color,
        status_enabled: n.status_enabled,
        position: n.position,
        status_position: n.status_position,
        area_mode: n.area_mode,
    }
}

#[cfg(test)]
mod tests {
    use crate::settings::test_support::test_vm;

    use super::*;

    #[test]
    fn notification_edits_round_trip() {
        let vm = test_vm();
        vm.notifications.set_warning_duration_ms(3_000).unwrap();
        vm.notifications.set_error_modal(false).unwrap();
        vm.notifications
            .set_position(NotificationPosition::TopRight)
            .unwrap();
        let view = vm.notifications.view.get();
        assert_eq!(view.warning_duration_ms, 3_000);
        assert!(!view.error_modal);
        assert_eq!(view.position, NotificationPosition::TopRight);
    }
}
