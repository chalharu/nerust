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

/// At most two field updates per event, inline on the stack: the input
/// hot path never heap-allocates for the common one-update case.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FieldUpdates {
    updates: [(usize, bool); 2],
    len: u8,
}

impl FieldUpdates {
    fn one(update: (usize, bool)) -> Self {
        Self {
            updates: [update, (0, false)],
            len: 1,
        }
    }

    fn two(first: (usize, bool), second: (usize, bool)) -> Self {
        Self {
            updates: [first, second],
            len: 2,
        }
    }

    pub(crate) fn as_slice(&self) -> &[(usize, bool)] {
        &self.updates[..self.len as usize]
    }
}

impl PartialEq<Vec<(usize, bool)>> for FieldUpdates {
    fn eq(&self, other: &Vec<(usize, bool)>) -> bool {
        self.as_slice() == other.as_slice()
    }
}

/// Live press state for every field.
#[derive(Debug, Default)]
pub(crate) struct FieldPressState {
    holders: HashMap<usize, HashSet<FieldSource>>,
    /// (attachment, control) → field, learned from live events.
    /// Copy ids: no allocation on the input hot path.
    control_field: HashMap<(AttachmentId, DigitalControlId), usize>,
    /// Pressed dpad controls per attachment and abstract.
    dpad_held: HashMap<AttachmentId, HashMap<AbstractKey, HashSet<DigitalControlId>>>,
    /// Axes held in opposition per attachment: both fields stay down.
    neutral: HashSet<(AttachmentId, DpadAxis)>,
}

impl FieldPressState {
    /// Press a field. `dpad` carries Copy ids plus the abstract for
    /// direction buttons. Returns (field, assert) updates, inline.
    pub(crate) fn press(
        &mut self,
        field: usize,
        source: FieldSource,
        dpad: Option<(AttachmentId, DigitalControlId, AbstractKey)>,
    ) -> FieldUpdates {
        self.holders.entry(field).or_default().insert(source);
        let Some((attachment, control, abs)) = dpad else {
            return FieldUpdates::one((field, true));
        };
        self.control_field.insert((attachment, control), field);
        let Some((axis, opposite)) = dpad_pair(abs) else {
            return FieldUpdates::one((field, true));
        };
        self.dpad_held
            .entry(attachment)
            .or_default()
            .entry(abs)
            .or_default()
            .insert(control);
        if self.is_abstract_held(attachment, opposite) {
            // Opposite held: neutralize the axis, both fields release.
            self.neutral.insert((attachment, axis));
            match self.peer_field(attachment, opposite) {
                Some(peer) => FieldUpdates::two((field, false), (peer, false)),
                None => FieldUpdates::one((field, false)),
            }
        } else {
            FieldUpdates::one((field, true))
        }
    }

    /// Drop one dpad hold. None only when the attachment holds nothing at
    /// all (the pair is unchanged); missing direction inside a held
    /// attachment counts as still held, as before.
    fn remove_dpad_hold(
        &mut self,
        attachment: AttachmentId,
        abs: AbstractKey,
        control: DigitalControlId,
    ) -> Option<bool> {
        let held = self.dpad_held.get_mut(&attachment)?;
        let Some(ctrls) = held.get_mut(&abs) else {
            return Some(true);
        };
        ctrls.remove(&control);
        let still_held = !ctrls.is_empty();
        if ctrls.is_empty() {
            held.remove(&abs);
        }
        if held.is_empty() {
            self.dpad_held.remove(&attachment);
        }
        Some(still_held)
    }

    /// Release a field. Returns (field, assert) updates to apply, inline.
    pub(crate) fn release(
        &mut self,
        field: usize,
        source: FieldSource,
        dpad: Option<(AttachmentId, DigitalControlId, AbstractKey)>,
    ) -> FieldUpdates {
        if let Some(holders) = self.holders.get_mut(&field) {
            holders.remove(&source);
            if holders.is_empty() {
                self.holders.remove(&field);
            }
        }
        let Some((attachment, control, abs)) = dpad else {
            return FieldUpdates::one((field, self.is_held(field)));
        };
        let Some((axis, opposite)) = dpad_pair(abs) else {
            return FieldUpdates::one((field, self.is_held(field)));
        };
        let side_still_held = self
            .remove_dpad_hold(attachment, abs, control)
            .unwrap_or(false);
        if side_still_held {
            // Same direction still held by another source: the pair is
            // unchanged, so a neutral pair stays neutral.
            if self.neutral.contains(&(attachment, axis)) {
                return FieldUpdates::one((field, false));
            }
            return FieldUpdates::one((field, self.is_held(field)));
        }
        if !self.neutral.contains(&(attachment, axis)) {
            return FieldUpdates::one((field, self.is_held(field)));
        }
        // This side fully left. Resolve the pair: the still-held opposite
        // re-asserts, or the axis simply unblocks.
        self.neutral.remove(&(attachment, axis));
        if self.is_abstract_held(attachment, opposite) {
            match self.peer_field(attachment, opposite) {
                Some(peer) => FieldUpdates::two((field, false), (peer, true)),
                None => FieldUpdates::one((field, false)),
            }
        } else {
            FieldUpdates::one((field, self.is_held(field)))
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

    fn is_abstract_held(&self, attachment: AttachmentId, abs: AbstractKey) -> bool {
        self.dpad_held
            .get(&attachment)
            .is_some_and(|held| held.get(&abs).is_some_and(|ctrls| !ctrls.is_empty()))
    }

    fn peer_field(&self, attachment: AttachmentId, abs: AbstractKey) -> Option<usize> {
        // Deterministic pick: several controls may hold one abstract, but
        // they converge on one field; take the smallest instead of HashSet
        // iteration order.
        self.dpad_held
            .get(&attachment)?
            .get(&abs)?
            .iter()
            .filter_map(|ctrl| self.control_field.get(&(attachment, *ctrl)).copied())
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATT: AttachmentId = AttachmentId::new("test.slot");
    const LEFT_CTRL: DigitalControlId = DigitalControlId::new("test.left");
    const RIGHT_CTRL: DigitalControlId = DigitalControlId::new("test.right");

    fn left() -> (AttachmentId, DigitalControlId, AbstractKey) {
        (ATT, LEFT_CTRL, AbstractKey::DpadLeft)
    }

    fn right() -> (AttachmentId, DigitalControlId, AbstractKey) {
        (ATT, RIGHT_CTRL, AbstractKey::DpadRight)
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
            ATT,
            DigitalControlId::new("test.left.b"),
            AbstractKey::DpadLeft,
        );
        assert_eq!(state.press(10, left_a, Some(left())), vec![(10, true)]);
        // Same direction, second source: still asserted, no neutral.
        assert_eq!(state.press(10, left_b, Some(left_b_ctrl)), vec![(10, true)]);
        // Opposite presses: neutral.
        assert_eq!(
            state.press(11, right_key, Some(right())),
            vec![(11, false), (10, false)]
        );
        // One left source lets go: left still held, pair stays neutral.
        assert_eq!(state.release(10, left_a, Some(left())), vec![(10, false)]);
        // Last left source lets go: right re-asserts.
        assert_eq!(
            state.release(10, left_b, Some(left_b_ctrl)),
            vec![(10, false), (11, true)]
        );
    }
}
