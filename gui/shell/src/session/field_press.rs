//! Per-field press bookkeeping: OR merge plus dpad-neutral resolution.
//!
//! Leaf module (trait imports only): the session resolves attachments and
//! abstracts, this machine only tracks holders. One field stays asserted
//! while any source holds it; opposite dpad directions on one attachment
//! neutralize each other (both fields release) until one side lets go,
//! at which point the still-held side re-asserts. No time dependence:
//! no last-wins, no first-wins.

use std::collections::{HashMap, HashSet};

use nerust_input_traits::{AbstractKey, AttachmentId, DigitalControlId, Key};

/// One holder of a field: a keyboard key or a physical control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FieldSource {
    Key(Key),
    Control(AttachmentId, DigitalControlId),
}

/// Dpad axis for neutral resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DpadAxis {
    Vertical,
    Horizontal,
}

/// Axis and opposite direction for a dpad abstract, if it is one.
/// Single table so callers never pair a separate axis check with an
/// `expect` on the opposite.
pub(crate) fn dpad_pair(abs: AbstractKey) -> Option<(DpadAxis, AbstractKey)> {
    match abs {
        AbstractKey::DpadUp => Some((DpadAxis::Vertical, AbstractKey::DpadDown)),
        AbstractKey::DpadDown => Some((DpadAxis::Vertical, AbstractKey::DpadUp)),
        AbstractKey::DpadLeft => Some((DpadAxis::Horizontal, AbstractKey::DpadRight)),
        AbstractKey::DpadRight => Some((DpadAxis::Horizontal, AbstractKey::DpadLeft)),
        _ => None,
    }
}

/// Live press state for every field.
#[derive(Debug, Default)]
pub(crate) struct FieldPressState {
    holders: HashMap<usize, HashSet<FieldSource>>,
    /// (attachment, control) → field, learned from live events.
    control_field: HashMap<(String, String), usize>,
    /// Pressed dpad controls per attachment and abstract.
    dpad_held: HashMap<String, HashMap<AbstractKey, HashSet<String>>>,
    /// Axes held in opposition per attachment: both fields stay down.
    neutral: HashSet<(String, DpadAxis)>,
}

impl FieldPressState {
    /// Press a field. `dpad` carries attachment, control id string, and
    /// abstract for direction buttons. Returns (field, assert) updates.
    pub(crate) fn press(
        &mut self,
        field: usize,
        source: FieldSource,
        dpad: Option<(String, String, AbstractKey)>,
    ) -> Vec<(usize, bool)> {
        self.holders.entry(field).or_default().insert(source);
        let Some((attachment, control, abs)) = dpad else {
            return vec![(field, true)];
        };
        self.control_field
            .insert((attachment.clone(), control.clone()), field);
        let Some((axis, opposite)) = dpad_pair(abs) else {
            return vec![(field, true)];
        };
        self.dpad_held
            .entry(attachment.clone())
            .or_default()
            .entry(abs)
            .or_default()
            .insert(control);
        if self.is_abstract_held(&attachment, opposite) {
            // Opposite held: neutralize the axis, both fields release.
            self.neutral.insert((attachment.clone(), axis));
            match self.peer_field(&attachment, opposite) {
                Some(peer) => vec![(field, false), (peer, false)],
                None => vec![(field, false)],
            }
        } else {
            vec![(field, true)]
        }
    }

    /// Release a field. Returns (field, assert) updates to apply.
    pub(crate) fn release(
        &mut self,
        field: usize,
        source: FieldSource,
        dpad: Option<(String, String, AbstractKey)>,
    ) -> Vec<(usize, bool)> {
        if let Some(holders) = self.holders.get_mut(&field) {
            holders.remove(&source);
            if holders.is_empty() {
                self.holders.remove(&field);
            }
        }
        let Some((attachment, control, abs)) = dpad else {
            return vec![(field, self.is_held(field))];
        };
        let Some((axis, opposite)) = dpad_pair(abs) else {
            return vec![(field, self.is_held(field))];
        };
        let mut side_still_held = false;
        if let Some(held) = self.dpad_held.get_mut(&attachment) {
            if let Some(ctrls) = held.get_mut(&abs) {
                ctrls.remove(&control);
                side_still_held = !ctrls.is_empty();
                if ctrls.is_empty() {
                    held.remove(&abs);
                }
            } else {
                side_still_held = true;
            }
            if held.is_empty() {
                self.dpad_held.remove(&attachment);
            }
        }
        if side_still_held {
            // Same direction still held by another source: the pair is
            // unchanged, so a neutral pair stays neutral.
            if self.neutral.contains(&(attachment, axis)) {
                return vec![(field, false)];
            }
            return vec![(field, self.is_held(field))];
        }
        if !self.neutral.contains(&(attachment.clone(), axis)) {
            return vec![(field, self.is_held(field))];
        }
        // This side fully left. Resolve the pair: the still-held opposite
        // re-asserts, or the axis simply unblocks.
        self.neutral.remove(&(attachment.clone(), axis));
        if self.is_abstract_held(&attachment, opposite) {
            match self.peer_field(&attachment, opposite) {
                Some(peer) => vec![(field, false), (peer, true)],
                None => vec![(field, false)],
            }
        } else {
            vec![(field, self.is_held(field))]
        }
    }

    /// Forget everything (focus loss; the emulator input clears with it).
    pub(crate) fn clear(&mut self) {
        self.holders.clear();
        self.control_field.clear();
        self.dpad_held.clear();
        self.neutral.clear();
    }

    fn is_held(&self, field: usize) -> bool {
        self.holders
            .get(&field)
            .is_some_and(|holders| !holders.is_empty())
    }

    fn is_abstract_held(&self, attachment: &str, abs: AbstractKey) -> bool {
        self.dpad_held
            .get(attachment)
            .is_some_and(|held| held.get(&abs).is_some_and(|ctrls| !ctrls.is_empty()))
    }

    fn peer_field(&self, attachment: &str, abs: AbstractKey) -> Option<usize> {
        let ctrl = self.dpad_held.get(attachment)?.get(&abs)?.iter().next()?;
        self.control_field
            .get(&(attachment.to_string(), ctrl.clone()))
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATT: &str = "test.slot";
    const LEFT_CTRL: &str = "test.left";
    const RIGHT_CTRL: &str = "test.right";

    fn left() -> (String, String, AbstractKey) {
        (
            ATT.to_string(),
            LEFT_CTRL.to_string(),
            AbstractKey::DpadLeft,
        )
    }

    fn right() -> (String, String, AbstractKey) {
        (
            ATT.to_string(),
            RIGHT_CTRL.to_string(),
            AbstractKey::DpadRight,
        )
    }

    #[test]
    fn same_field_or_merges_sources() {
        let mut state = FieldPressState::default();
        let key_a = FieldSource::Key(Key::KeyA);
        let key_b = FieldSource::Key(Key::KeyB);
        assert_eq!(state.press(3, key_a, None), vec![(3, true)]);
        // Second source, same field: stays asserted.
        assert_eq!(state.press(3, key_b, None), vec![(3, true)]);
        // First release: still held by the second source.
        assert_eq!(state.release(3, key_a, None), vec![(3, true)]);
        assert_eq!(state.release(3, key_b, None), vec![(3, false)]);
    }

    #[test]
    fn opposite_directions_neutralize_until_release() {
        let mut state = FieldPressState::default();
        let left_key = FieldSource::Key(Key::ArrowLeft);
        let right_key = FieldSource::Key(Key::ArrowRight);
        // Left field 10, right field 11.
        assert_eq!(state.press(10, left_key, Some(left())), vec![(10, true)]);
        // Right presses while left is held: both release.
        assert_eq!(
            state.press(11, right_key, Some(right())),
            vec![(11, false), (10, false)]
        );
        // Left lets go while right is still held: right re-asserts.
        assert_eq!(
            state.release(10, left_key, Some(left())),
            vec![(10, false), (11, true)]
        );
        // Right releases last: nothing held.
        assert_eq!(
            state.release(11, right_key, Some(right())),
            vec![(11, false)]
        );
    }

    #[test]
    fn partial_release_keeps_neutral_while_side_held() {
        let mut state = FieldPressState::default();
        let left_a = FieldSource::Key(Key::KeyA);
        let left_b = FieldSource::Key(Key::KeyB);
        let right_key = FieldSource::Key(Key::ArrowRight);
        let left_b_ctrl = (
            ATT.to_string(),
            "test.left.b".to_string(),
            AbstractKey::DpadLeft,
        );
        assert_eq!(state.press(10, left_a, Some(left())), vec![(10, true)]);
        // Same direction, second source: still asserted, no neutral.
        assert_eq!(
            state.press(10, left_b, Some(left_b_ctrl.clone())),
            vec![(10, true)]
        );
        // Opposite presses: neutral.
        assert_eq!(
            state.press(11, right_key, Some(right())),
            vec![(11, false), (10, false)]
        );
        // One left source lets go: left still held, pair stays neutral.
        assert_eq!(state.release(10, left_a, Some(left())), vec![(10, false)]);
        // Last left source lets go: right re-asserts.
        assert_eq!(
            state.release(10, left_b, Some(left_b_ctrl.clone())),
            vec![(10, false), (11, true)]
        );
    }
}
