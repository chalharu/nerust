use panechord_runtime::WindowEvent;

pub(crate) fn event(kind: i32, value: &str) -> Option<WindowEvent> {
    match kind {
        4 => Some(WindowEvent::Focused(value == "1")),
        5..=7 => Some(WindowEvent::KeyboardInput {
            key: panechord_runtime::native_keys::macos(value.parse().ok()?),
            pressed: kind != 6,
            repeat: kind == 7,
            synthetic: false,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panechord_runtime::KeyCode;
    #[test]
    fn appkit_events_keep_physical_key_repeat_release_and_focus() {
        for (kind, pressed, repeat) in [(5, true, false), (6, false, false), (7, true, true)] {
            let Some(WindowEvent::KeyboardInput {
                key,
                pressed: actual,
                repeat: repeated,
                synthetic,
            }) = event(kind, "9")
            else {
                panic!("missing key event")
            };
            assert_eq!(key, KeyCode::KeyV);
            assert_eq!(actual, pressed);
            assert_eq!(repeated, repeat);
            assert!(!synthetic);
        }
        assert!(matches!(event(4, "0"), Some(WindowEvent::Focused(false))));
        assert!(event(5, "invalid").is_none());
    }
}
