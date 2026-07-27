use super::{
    dto::{AudioView, ChoiceView},
    editor::{SettingsEditor, ViewModelError},
    property::ReadOnlyObservableProperty,
};

#[derive(Clone)]
pub struct AudioSettingsViewModel {
    _editor: SettingsEditor,
    pub view: ReadOnlyObservableProperty<AudioView>,
}

impl AudioSettingsViewModel {
    pub fn new(editor: &SettingsEditor) -> Self {
        let current = editor.current();
        let initial = project_view(&current);
        drop(current);
        let view = editor
            .projections()
            .register("audio", initial, project_view);
        Self {
            _editor: editor.clone(),
            view,
        }
    }

    pub fn set_mute(&self, value: bool) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.audio.muted = value;
            Ok(())
        })
    }

    pub fn set_volume(&self, value: u8) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.audio.master_volume_percent = value;
            Ok(())
        })
    }

    pub fn set_sample_rate(&self, value: u32) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.audio.sample_rate = value;
            Ok(())
        })
    }

    pub fn set_latency(&self, value: u16) -> Result<(), ViewModelError> {
        self._editor.transact(|state| {
            state.draft_mut().local.audio.latency_ms = value;
            Ok(())
        })
    }
}

fn project_view(state: &super::EditorState) -> AudioView {
    let rates: &[u32] = if state.supported_sample_rates.is_empty() {
        &[44_100, 48_000]
    } else {
        &state.supported_sample_rates
    };
    AudioView {
        muted: state.draft.local.audio.muted,
        volume_percent: state.draft.local.audio.master_volume_percent,
        sample_rate: state.draft.local.audio.sample_rate,
        sample_rate_choices: rates
            .iter()
            .map(|&r| ChoiceView {
                value: r,
                label: format!("{r}"),
            })
            .collect(),
        latency_ms: state.draft.local.audio.latency_ms,
    }
}

#[cfg(test)]
mod tests {
    use crate::settings::test_support::test_vm;

    #[test]
    fn set_mute_updates_projection() {
        let vm = test_vm();
        vm.audio.set_mute(true).unwrap();
        let view = vm.audio.view.get();
        assert!(view.muted);
    }

    #[test]
    fn set_volume_updates_projection() {
        let vm = test_vm();
        vm.audio.set_volume(42).unwrap();
        let view = vm.audio.view.get();
        assert_eq!(view.volume_percent, 42);
    }

    #[test]
    fn set_sample_rate_updates_projection() {
        let vm = test_vm();
        vm.audio.set_sample_rate(44_100).unwrap();
        let view = vm.audio.view.get();
        assert_eq!(view.sample_rate, 44_100);
    }

    #[test]
    fn set_latency_updates_projection() {
        let vm = test_vm();
        vm.audio.set_latency(75).unwrap();
        let view = vm.audio.view.get();
        assert_eq!(view.latency_ms, 75);
    }

    #[test]
    fn set_same_mute_is_noop() {
        let vm = test_vm();
        let rev_before = vm.revision.get();
        vm.audio.set_mute(false).unwrap();
        assert_eq!(vm.revision.get(), rev_before, "revision should not advance");
    }
}
