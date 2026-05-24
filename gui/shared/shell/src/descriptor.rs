use nerust_console::Console;
use nerust_gui_runtime::session::GuiSession;
use nerust_gui_session::core::SessionCore;
use nerust_input_nes::topology::input_topology_descriptor;
use nerust_input_schema::InputTopologyDescriptor;
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_sound_openal::OpenAl;
use nerust_sound_traits::{MixerInput, Sound};
use nerust_timer::CLOCK_RATE;

#[derive(Debug, Clone, Copy, Default)]
pub struct NesConsoleProfile;

impl NesConsoleProfile {
    pub fn build_console(self) -> Console {
        self.build_console_with(
            OpenAl::new(48_000, CLOCK_RATE as i32, 128, 20),
            ScreenBuffer::new_nes_gpu_default(),
        )
    }

    pub fn build_console_with<S: 'static + Sound + MixerInput + Send>(
        self,
        speaker: S,
        screen_buffer: ScreenBuffer,
    ) -> Console {
        Console::new(speaker, screen_buffer)
    }

    pub fn build_gui_session(self) -> GuiSession {
        GuiSession::from_session_core(SessionCore::from_console(self.build_console()))
    }

    pub fn input_topology_descriptor(&self) -> InputTopologyDescriptor {
        input_topology_descriptor()
    }
}

#[cfg(test)]
mod tests {
    use super::NesConsoleProfile;
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
        assert!(!player_two_controls.iter().any(|control| {
            matches!(
                control,
                ControlDescriptor::Digital(digital) if digital.id == NES_CONTROL_SELECT
            )
        }));
    }
}
