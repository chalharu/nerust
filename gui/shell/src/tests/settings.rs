use nerust_core_traits::audio::{AudioBackend, NullAudio};
use nerust_gui_settings::local::ScalingMode;

use super::scaling_factor;

#[test]
fn null_audio_reports_default_sample_rate() {
    let mut speaker = NullAudio;

    AudioBackend::start(&mut speaker);
    AudioBackend::push(&mut speaker, 0.5);
    AudioBackend::pause(&mut speaker);

    assert_eq!(AudioBackend::sample_rate(&speaker), 48_000);
}

#[test]
fn scaling_factor_uses_none_for_fit_mode() {
    assert_eq!(scaling_factor(ScalingMode::FitToWindow), None);
    assert_eq!(scaling_factor(ScalingMode::X4), Some(4));
}
