use nerust_keyboard::Key;

pub(crate) fn gdk_key_to_code(key: gdk::Key) -> Option<Key> {
    Some(Key::from(key))
}
