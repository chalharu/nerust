//! Desktop pad reader: HID snapshots in, normalized pad events out.
//!
//! The manager owns connection slots and edge-detects every frame; the
//! session owns the OR-merge, port assignment, and repeat suppression, so
//! this layer never tracks press state beyond its last emitted frame.
//! Unknown VID/PID pairs ride the SDL-standard positional fallback until
//! the DB learns them (Phase 3); known pads invert their DB table.

#[cfg(target_os = "macos")]
mod macos;

use nerust_gamepad_db::RawMapping;
use nerust_input_traits::{GamepadAxis, StandardButton};

/// One raw HID frame: SDL-indexed physical state. The macOS backend builds
/// these; the mapper below turns them into positions.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Stable identity (`HID:vid:pid:location:serial`): reconnects to the
    /// same port hash to the same descriptor, so order survives reuse.
    pub descriptor: String,
    pub vid: u16,
    pub pid: u16,
    /// Button `bN` pressed state.
    pub buttons: Vec<bool>,
    /// Axis `aN` values, already `-1.0..=1.0` (session deadzones later).
    pub axes: Vec<f32>,
    /// Hat `hN` SDL masks (`1/2/4/8` = up/right/down/left).
    pub hats: Vec<u8>,
}

/// Normalized event for one connection-order device.
#[derive(Debug, Clone, PartialEq)]
pub enum PadEvent {
    Connected {
        descriptor: String,
    },
    Disconnected,
    Button {
        button: StandardButton,
        pressed: bool,
    },
    Axis {
        axis: GamepadAxis,
        value: f32,
    },
}

/// DB inversion with the SDL-standard positional fallback. Unknown pads
/// keep flowing (hat 0 drives dpad) instead of parking silently.
#[derive(Debug, Clone, Copy)]
struct Mapper {
    mapping: Option<RawMapping>,
}

impl Mapper {
    fn new(vid: u16, pid: u16) -> Self {
        Self {
            mapping: nerust_gamepad_db::resolve(vid, pid).copied(),
        }
    }

    fn button(self, physical: u8) -> Option<StandardButton> {
        self.mapping
            .and_then(|mapping| mapping.button_for_index(physical))
            .or_else(|| StandardButton::from_index(usize::from(physical)))
    }

    fn axis(self, physical: u8) -> Option<GamepadAxis> {
        self.mapping
            .and_then(|mapping| mapping.axis_for_index(physical))
            .or_else(|| GamepadAxis::from_index(usize::from(physical)))
    }

    /// Logical buttons for one hat state: DB entries first, hat 0 dpad
    /// fallback for unknown pads, nothing for higher unknown hats.
    fn hat_buttons(self, hat: u8, value: u8, out: &mut Vec<StandardButton>) {
        if let Some(mapping) = self.mapping {
            mapping.hat_buttons(hat, value, out);
        } else if hat == 0 {
            const DPAD: [(u8, StandardButton); 4] = [
                (1, StandardButton::DpadUp),
                (2, StandardButton::DpadRight),
                (4, StandardButton::DpadDown),
                (8, StandardButton::DpadLeft),
            ];
            out.extend(
                DPAD.iter()
                    .filter(|(mask, _)| value & mask != 0)
                    .map(|(_, button)| *button),
            );
        }
    }
}

/// One connection slot: stable id, identity, and the last emitted frame
/// the next snapshot diffs against.
struct Slot {
    id: u32,
    descriptor: String,
    mapper: Mapper,
    last_buttons: u32,
    last_axes: [f32; GamepadAxis::COUNT],
}

/// Connection slots plus edge detection. Snapshots arrive per poll from
/// the OS backend (`macos`); everything here is pure and unit-tested.
pub struct Manager {
    slots: Vec<Slot>,
    next_id: u32,
    #[cfg(target_os = "macos")]
    backend: macos::Backend,
}

impl Manager {
    /// Backend open failure parks as empty (warned once at construction),
    /// never as a constructor error the session must thread through.
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            next_id: 0,
            #[cfg(target_os = "macos")]
            backend: macos::Backend::new(),
        }
    }

    /// Drain one OS poll into normalized events. Caller-owned buffer, so
    /// polling never allocates past the snapshot the backend builds.
    pub fn drain(&mut self, out: &mut Vec<(u32, PadEvent)>) {
        #[cfg(target_os = "macos")]
        {
            let snapshots = self.backend.poll();
            self.update(&snapshots, out);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = out;
        }
    }

    /// Fold snapshots into events: connects, edge-diffed buttons and
    /// axes, and release-synthesizing disconnects. Missing snapshots mean
    /// vanished devices; their last frame converges here, never silently.
    fn update(&mut self, snapshots: &[Snapshot], out: &mut Vec<(u32, PadEvent)>) {
        for snapshot in snapshots {
            let position = self
                .slots
                .iter()
                .position(|slot| slot.descriptor == snapshot.descriptor);
            let index = match position {
                Some(index) => index,
                None => {
                    let id = self.next_id;
                    self.next_id += 1;
                    self.slots.push(Slot {
                        id,
                        descriptor: snapshot.descriptor.clone(),
                        mapper: Mapper::new(snapshot.vid, snapshot.pid),
                        last_buttons: 0,
                        last_axes: [0.0; GamepadAxis::COUNT],
                    });
                    out.push((
                        id,
                        PadEvent::Connected {
                            descriptor: snapshot.descriptor.clone(),
                        },
                    ));
                    self.slots.len() - 1
                }
            };
            Self::diff_frame(&mut self.slots[index], snapshot, out);
        }
        let mut gone = 0;
        while gone < self.slots.len() {
            if snapshots
                .iter()
                .any(|snapshot| snapshot.descriptor == self.slots[gone].descriptor)
            {
                gone += 1;
                continue;
            }
            let slot = self.slots.remove(gone);
            for index in 0..StandardButton::COUNT {
                if slot.last_buttons & (1 << index) != 0
                    && let Some(button) = StandardButton::from_index(index)
                {
                    out.push((
                        slot.id,
                        PadEvent::Button {
                            button,
                            pressed: false,
                        },
                    ));
                }
            }
            for (index, value) in slot.last_axes.iter().copied().enumerate() {
                if value != 0.0
                    && let Some(axis) = GamepadAxis::from_index(index)
                {
                    out.push((slot.id, PadEvent::Axis { axis, value: 0.0 }));
                }
            }
            out.push((slot.id, PadEvent::Disconnected));
        }
    }

    /// Edge-diff one frame through the slot's mapper. Releases lead
    /// presses so a dpad reversal never holds both directions.
    fn diff_frame(slot: &mut Slot, snapshot: &Snapshot, out: &mut Vec<(u32, PadEvent)>) {
        let mut current: u32 = 0;
        for (physical, pressed) in snapshot.buttons.iter().copied().enumerate() {
            let Ok(physical) = u8::try_from(physical) else {
                break;
            };
            if pressed && let Some(button) = slot.mapper.button(physical) {
                current |= 1 << button.index();
            }
        }
        let mut hats = Vec::new();
        for (hat, value) in snapshot.hats.iter().copied().enumerate() {
            let Ok(hat) = u8::try_from(hat) else {
                break;
            };
            hats.clear();
            slot.mapper.hat_buttons(hat, value, &mut hats);
            for button in hats.drain(..) {
                current |= 1 << button.index();
            }
        }
        let changed = slot.last_buttons ^ current;
        for index in 0..StandardButton::COUNT {
            if changed & (1 << index) != 0
                && let Some(button) = StandardButton::from_index(index)
            {
                out.push((
                    slot.id,
                    PadEvent::Button {
                        button,
                        pressed: current & (1 << index) != 0,
                    },
                ));
            }
        }
        slot.last_buttons = current;
        for (physical, value) in snapshot.axes.iter().copied().enumerate() {
            let Ok(physical) = u8::try_from(physical) else {
                break;
            };
            if let Some(axis) = slot.mapper.axis(physical)
                && slot.last_axes[axis.index()] != value
            {
                slot.last_axes[axis.index()] = value;
                out.push((slot.id, PadEvent::Axis { axis, value }));
            }
        }
    }
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(descriptor: &str, vid: u16, pid: u16) -> Snapshot {
        Snapshot {
            descriptor: descriptor.to_owned(),
            vid,
            pid,
            buttons: Vec::new(),
            axes: Vec::new(),
            hats: Vec::new(),
        }
    }

    #[test]
    fn connect_diff_disconnect_round_trip() {
        let mut manager = Manager::new();
        let mut out = Vec::new();
        // Unknown VID/PID: SDL-standard fallback positions apply.
        let mut connected = snapshot("pad-a", 0x1234, 0x5678);
        connected.buttons = vec![true, false, false, false];
        connected.axes = vec![0.0, 0.5];
        manager.update(std::slice::from_ref(&connected), &mut out);
        assert!(out.contains(&(
            0,
            PadEvent::Connected {
                descriptor: "pad-a".to_owned()
            }
        )));
        assert!(out.contains(&(
            0,
            PadEvent::Button {
                button: StandardButton::South,
                pressed: true,
            }
        )));
        assert!(out.contains(&(
            0,
            PadEvent::Axis {
                axis: GamepadAxis::LeftStickY,
                value: 0.5,
            }
        )));
        // Steady state emits nothing; release flips the one bit.
        out.clear();
        manager.update(std::slice::from_ref(&connected), &mut out);
        assert!(out.is_empty());
        let mut released = connected.clone();
        released.buttons[0] = false;
        released.axes[1] = 0.0;
        manager.update(std::slice::from_ref(&released), &mut out);
        assert!(out.contains(&(
            0,
            PadEvent::Button {
                button: StandardButton::South,
                pressed: false,
            }
        )));
        assert!(out.contains(&(
            0,
            PadEvent::Axis {
                axis: GamepadAxis::LeftStickY,
                value: 0.0,
            }
        )));
        // Vanishing converges held state, then disconnects.
        out.clear();
        let mut held = released.clone();
        held.buttons[1] = true;
        manager.update(std::slice::from_ref(&held), &mut out);
        assert_eq!(out.len(), 1);
        out.clear();
        manager.update(&[], &mut out);
        assert!(out.contains(&(
            0,
            PadEvent::Button {
                button: StandardButton::East,
                pressed: false,
            }
        )));
        assert!(out.contains(&(0, PadEvent::Disconnected)));
    }

    #[test]
    fn known_pad_inverts_its_db_table() {
        // Xbox 360 (all platform tables): `b1` is East, `h0.1` is D-Up.
        let mut manager = Manager::new();
        let mut connected = snapshot("pad-x", 0x045e, 0x028e);
        connected.buttons = vec![false, true];
        connected.hats = vec![1];
        let mut out = Vec::new();
        manager.update(std::slice::from_ref(&connected), &mut out);
        assert!(out.contains(&(
            0,
            PadEvent::Button {
                button: StandardButton::East,
                pressed: true,
            }
        )));
        assert!(out.contains(&(
            0,
            PadEvent::Button {
                button: StandardButton::DpadUp,
                pressed: true,
            }
        )));
        // Unknown hats on unknown pads stay silent, never guessed.
        out.clear();
        let mut manager = Manager::new();
        let mut unknown = snapshot("pad-u", 0x9999, 0x9999);
        unknown.hats = vec![0, 1];
        manager.update(std::slice::from_ref(&unknown), &mut out);
        assert!(
            !out.iter().any(|(_, event)| matches!(
                event,
                PadEvent::Button { button, .. } if *button == StandardButton::DpadUp
            )),
            "second-hat fallback must stay silent, got {out:?}"
        );
    }

    #[test]
    fn reconnect_keeps_its_stable_id() {
        let mut manager = Manager::new();
        let mut out = Vec::new();
        manager.update(&[snapshot("pad-a", 0, 0)], &mut out);
        manager.update(&[], &mut out);
        out.clear();
        manager.update(&[snapshot("pad-a", 0, 0)], &mut out);
        // Fresh slot, fresh id: the session order (not the manager id)
        // is what survives reconnects via the descriptor hash.
        assert!(out.contains(&(
            1,
            PadEvent::Connected {
                descriptor: "pad-a".to_owned()
            }
        )));
    }
}
