use nerust_core_traits::identity::SystemId;
use nerust_gui_settings::{input::KeyboardKey, shared::DesktopSharedSettings};
use nerust_input_traits::DigitalInputEvent;

pub fn controller_event_for_key<F>(
    settings: &DesktopSharedSettings,
    system: SystemId,
    key: KeyboardKey,
    pressed: bool,
    resolve: F,
) -> Option<DigitalInputEvent>
where
    F: Fn(&str, &str, bool) -> Option<DigitalInputEvent>,
{
    let profile = settings
        .input
        .systems
        .get(&system)?
        .implicit_keyboard_profile()?;
    profile
        .bindings
        .iter()
        .find(|binding| binding.key == key)
        .and_then(|binding| {
            resolve(
                binding.attachment.as_str(),
                binding.control.as_str(),
                pressed,
            )
        })
}

#[cfg(test)]
#[path = "../../../tests/settings/bindings/events/controller.rs"]
mod tests;
