use crate::load::{NesLoadOptions, NesMmc3IrqVariant};
use crate::session::NesSession;
use crate::session::input::NesButton;
use nerust_gui_runtime::session::GuiSession;
use nerust_gui_session::core::SessionCore;
use nerust_input_nes::codec::decode_input_state;
use nerust_input_nes::frame::{Buttons, NesInputFrame};
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_sound_traits::{MixerInput, Sound};

#[derive(Default)]
struct TestSpeaker;

impl Sound for TestSpeaker {
    fn start(&mut self) {}

    fn pause(&mut self) {}
}

impl MixerInput for TestSpeaker {
    fn push(&mut self, _data: f32) {}
}

fn test_session() -> NesSession {
    NesSession::from_gui_session(GuiSession::from_session_core(SessionCore::from_console(
        nerust_console::Console::new(TestSpeaker, ScreenBuffer::new_nes_gpu_default()),
    )))
}

#[test]
fn nes_session_flushes_digital_input_into_controller_state() {
    let mut session = test_session();

    session.handle_player_one_button(NesButton::A, true);

    let frame = decode_input_state(
        &session
            .session
            .current_input_state()
            .expect("input state should export"),
    )
    .expect("input state should decode");
    assert_eq!(
        frame,
        NesInputFrame {
            player_one: Buttons::A,
            player_two: Buttons::empty(),
            microphone: false,
        }
    );
}

#[test]
fn nes_load_options_flow_into_session_load() {
    let mut session = test_session();
    let mut rom = vec![
        0x4E, 0x45, 0x53, 0x1A, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];
    rom.resize(16 + 0x8000 + 0x2000, 0);

    assert!(session.load_with_options(
        None,
        rom,
        NesLoadOptions {
            mmc3_irq_variant: Some(NesMmc3IrqVariant::Sharp),
        },
    ));
}
