use crate::settings::nes::{build_screen_buffer, build_speaker};
use crate::{load::NesLoadOptions, settings::nes::effective_load_options};
use nerust_console::Console;
use nerust_contract_options::CoreOptions;
use nerust_gui_runtime::session::GuiSession;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_session::core::SessionCore;
use nerust_input_nes::topology::input_topology_descriptor;
use nerust_input_schema::{InputTopologyDescriptor, SystemId};
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_sound_traits::{MixerInput, Sound};

pub trait SystemSessionProfile: Copy + Default {
    type LoadOptions: Clone + Default;

    fn build_gui_session(self, settings: &SettingsSnapshot) -> GuiSession;
    fn input_topology_descriptor(&self) -> InputTopologyDescriptor;
    fn system_id(self) -> SystemId;
    fn effective_load_options(
        self,
        settings: &SettingsSnapshot,
        explicit: Self::LoadOptions,
    ) -> CoreOptions;
    fn effective_rebuild_load_options(
        self,
        current_settings: &SettingsSnapshot,
        next_settings: &SettingsSnapshot,
        explicit: Self::LoadOptions,
    ) -> CoreOptions;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NesConsoleProfile;

impl NesConsoleProfile {
    pub fn build_console(self, settings: &SettingsSnapshot) -> Console {
        self.build_console_with(
            build_speaker(&settings.local),
            build_screen_buffer(&settings.shared),
        )
    }

    pub fn build_console_with<S: 'static + Sound + MixerInput + Send>(
        self,
        speaker: S,
        screen_buffer: ScreenBuffer,
    ) -> Console {
        Console::new(
            speaker,
            screen_buffer,
            nerust_input_nes_runtime::standard_controller_runtime(),
        )
    }

    pub fn build_gui_session(self, settings: &SettingsSnapshot) -> GuiSession {
        GuiSession::from_session_core(SessionCore::from_console(self.build_console(settings)))
    }

    pub fn input_topology_descriptor(&self) -> InputTopologyDescriptor {
        input_topology_descriptor()
    }
}

impl SystemSessionProfile for NesConsoleProfile {
    type LoadOptions = NesLoadOptions;

    fn build_gui_session(self, settings: &SettingsSnapshot) -> GuiSession {
        Self::build_gui_session(self, settings)
    }

    fn input_topology_descriptor(&self) -> InputTopologyDescriptor {
        Self::input_topology_descriptor(self)
    }

    fn system_id(self) -> SystemId {
        SystemId::Nes
    }

    fn effective_load_options(
        self,
        settings: &SettingsSnapshot,
        explicit: Self::LoadOptions,
    ) -> CoreOptions {
        effective_load_options(&settings.shared, explicit).into_core_options()
    }

    fn effective_rebuild_load_options(
        self,
        current_settings: &SettingsSnapshot,
        _next_settings: &SettingsSnapshot,
        explicit: Self::LoadOptions,
    ) -> CoreOptions {
        // Rebuilds refresh immediate host/runtime changes while preserving the
        // load-time core behavior of the currently running ROM.
        effective_load_options(&current_settings.shared, explicit).into_core_options()
    }
}

#[cfg(test)]
mod tests {
    use super::NesConsoleProfile;
    use crate::settings::defaults::manager::current_or_default;
    use crate::settings::defaults::seed::{
        default_app_state, default_local_settings, default_shared_settings,
    };
    use nerust_gui_runtime::settings::SettingsManager;
    use nerust_input_nes::topology::{
        FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
        NES_CONTROL_A, NES_CONTROL_SELECT, NES_DEVICE_PLAYER_ONE_PAD,
        NES_DEVICE_PLAYER_TWO_FAMICOM_PAD,
    };
    use nerust_input_schema::ControlDescriptor;

    #[test]
    fn nes_profile_reports_distinct_player_one_and_player_two_devices() {
        let descriptor = NesConsoleProfile.input_topology_descriptor();

        assert_eq!(descriptor.ports.len(), 2);
        assert_eq!(
            descriptor
                .attachment(NES_ATTACHMENT_PLAYER_ONE)
                .unwrap()
                .device,
            NES_DEVICE_PLAYER_ONE_PAD
        );
        assert_eq!(
            descriptor
                .attachment(NES_ATTACHMENT_PLAYER_TWO)
                .unwrap()
                .device,
            NES_DEVICE_PLAYER_TWO_FAMICOM_PAD
        );
    }

    #[test]
    fn nes_profile_keeps_select_on_player_one_and_microphone_on_player_two() {
        let descriptor = NesConsoleProfile.input_topology_descriptor();
        let player_one_controls = &descriptor
            .device(NES_DEVICE_PLAYER_ONE_PAD)
            .unwrap()
            .controls;
        let player_two_controls = &descriptor
            .device(NES_DEVICE_PLAYER_TWO_FAMICOM_PAD)
            .unwrap()
            .controls;

        assert!(player_one_controls.iter().any(|control| {
            matches!(
                control,
                ControlDescriptor::Digital(digital) if digital.id == NES_CONTROL_A
            )
        }));
        assert!(player_one_controls.iter().any(|control| {
            matches!(
                control,
                ControlDescriptor::Digital(digital) if digital.id == NES_CONTROL_SELECT
            )
        }));
        assert!(player_two_controls.iter().any(|control| {
            matches!(
                control,
                ControlDescriptor::Digital(digital) if digital.id == FAMICOM_P2_CONTROL_MICROPHONE
            )
        }));
    }

    #[test]
    fn gui_session_build_uses_saved_settings() {
        let settings = current_or_default(&SettingsManager::ephemeral(
            default_shared_settings(),
            default_local_settings(),
            default_app_state(),
        ));
        let session = NesConsoleProfile.build_gui_session(&settings);
        assert!(session.window_size().width > 0.0);
    }
}
