//! Controller profiles, slot assignments, and the input factory plan.

use std::{collections::HashMap, rc::Rc};

use super::{
    events::InputStateBuffer,
    ids::{AttachmentId, ControlId, DeviceKindId, ProfileId},
};

/// Unified session field map: (attachment, control) → absolute field.
/// End-to-end lookup key shared by factories and shells.
pub type UnifiedFieldMap = HashMap<(AttachmentId, ControlId), usize>;

/// Factory minting one input buffer per call; shared by every endpoint.
pub type BufferFactory = Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>;

/// A set of port identifiers a controller can occupy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortSet {
    pub ports: &'static [AttachmentId],
}

/// Identifies a single slot on the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotInfo {
    pub id: AttachmentId,
    pub label: &'static str,
}

impl SlotInfo {
    /// Check whether this slot corresponds to the given persisted ID string.
    pub fn matches_id(&self, id: &str) -> bool {
        self.id.as_str() == id
    }
}

/// Describes one control on a controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlInfo {
    pub id: ControlId,
    pub label: &'static str,
    pub kind: ControlKind,
    pub abstract_key: Option<AbstractKey>,
}

/// Classification of a control's physical behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    Digital,
    Analog,
    AnalogStick { clickable: bool },
    Mouse,
}

/// System-agnostic logical key identifier for default binding resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AbstractKey {
    Button1,
    Button2,
    Button3,
    Button4,
    Button5,
    Button6,
    Button7,
    Button8,
    Select,
    Start,
    Guide,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    Axis1X,
    Axis1Y,
    Axis2X,
    Axis2Y,
}

/// Describes one controller type (metadata sent to Frontend).
pub trait ControllerProfile: std::fmt::Debug + Send + Sync {
    fn profile_id(&self) -> ProfileId;
    fn label(&self) -> &'static str;
    fn port_sets(&self) -> &[PortSet];
    fn port_groups(&self) -> &[&[ControlInfo]];

    /// Device kind identifier for a given port group index.
    /// Override when port groups in one profile are different device kinds.
    fn device_kind_for_group(&self, _group_index: usize) -> DeviceKindId {
        DeviceKindId::new(self.profile_id().as_str())
    }
}

/// System port layout query. Factory → Frontend.
/// `slots()`/`controllers()` describe init-time topology: build once and cache.
pub trait InputPorts: std::fmt::Debug {
    fn slots(&self) -> &[SlotInfo];
    /// All controller profiles this system supports.
    /// Init-only: call during setup and cache; never poll on the hot path.
    fn controllers(&self) -> Box<dyn Iterator<Item = Rc<dyn ControllerProfile>> + '_>;

    /// Resolve a persisted slot ID string to an AttachmentId.
    /// Returns None if no slot matches.
    fn resolve_slot(&self, id: &str) -> Option<AttachmentId> {
        self.slots().iter().find(|s| s.matches_id(id)).map(|s| s.id)
    }

    /// Resolve a persisted controller ID string to a ControllerProfile.
    fn resolve_controller(&self, id: &str) -> Option<Rc<dyn ControllerProfile>> {
        self.controllers().find(|p| p.profile_id().as_str() == id)
    }
}

/// Slot-to-controller assignments.
/// Equality is order-sensitive and structural (slot order + [`ProfileId`] compare).
#[derive(Clone)]
pub struct InputAssignments {
    slots: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)>,
}

impl InputAssignments {
    pub fn new(slots: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)>) -> Self {
        Self { slots }
    }

    pub fn slots(&self) -> &[(AttachmentId, Option<Rc<dyn ControllerProfile>>)] {
        &self.slots
    }

    /// Validate once and place (registered profile, port-group index) at each
    /// physical slot. Canonical factory metadata, not caller-supplied metadata,
    /// determines topology. Missing assignments remain unconnected holes.
    pub fn placement_plan(
        &self,
        ports: &dyn InputPorts,
    ) -> Result<Vec<Option<(ProfileId, usize)>>, CreateSplitError> {
        let slots = ports.slots();
        let mut seen = std::collections::HashSet::new();
        let mut plan = vec![None; slots.len()];
        for (slot, profile) in &self.slots {
            if !slots.iter().any(|s| s.id == *slot) {
                return Err(CreateSplitError::SlotNotFound { slot: *slot });
            }
            if !seen.insert(*slot) {
                return Err(CreateSplitError::SlotConflict { a: *slot, b: *slot });
            }
            let Some(profile) = profile else { continue };
            let controller = profile.profile_id();
            let canonical = ports
                .resolve_controller(controller.as_str())
                .ok_or(CreateSplitError::ControllerNotFound { controller })?;
            let set = canonical
                .port_sets()
                .iter()
                .find(|set| set.ports.first() == Some(slot))
                .ok_or(CreateSplitError::IncompatibleController {
                    controller,
                    slot: *slot,
                })?;
            if set.ports.len() != canonical.port_groups().len() {
                return Err(CreateSplitError::IncompatibleController {
                    controller,
                    slot: *slot,
                });
            }
            for (group, port) in set.ports.iter().enumerate() {
                let index = slots
                    .iter()
                    .position(|s| s.id == *port)
                    .ok_or(CreateSplitError::SlotNotFound { slot: *port })?;
                if plan[index].is_some() {
                    return Err(CreateSplitError::SlotConflict { a: *slot, b: *port });
                }
                plan[index] = Some((controller, group));
            }
        }
        Ok(plan)
    }
}

impl std::fmt::Debug for InputAssignments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputAssignments")
            .field(
                "slots",
                &self
                    .slots
                    .iter()
                    .map(|(s, c)| (s, c.as_ref().map(|p| p.profile_id())))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Order-sensitive structural equality: slot ids and controller profile ids
/// compared pairwise; the persisted projection must preserve slot order.
impl PartialEq for InputAssignments {
    fn eq(&self, other: &Self) -> bool {
        self.slots.len() == other.slots.len()
            && self.slots.iter().zip(other.slots.iter()).all(|(a, b)| {
                a.0 == b.0
                    && match (&a.1, &b.1) {
                        (None, None) => true,
                        (Some(x), Some(y)) => x.profile_id() == y.profile_id(),
                        _ => false,
                    }
            })
    }
}

impl Eq for InputAssignments {}

/// Errors from create_split.
/// All identifiers are typed; empty yields `ControllerNotFound`, wrong count yields `IncompatibleController`.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CreateSplitError {
    #[error("slot '{slot}' not found")]
    SlotNotFound { slot: AttachmentId },
    #[error("controller '{controller}' not found")]
    ControllerNotFound { controller: ProfileId },
    #[error("controller '{controller}' cannot be assigned to slot '{slot}'")]
    IncompatibleController {
        controller: ProfileId,
        slot: AttachmentId,
    },
    #[error("slot conflict between {a} and {b}")]
    SlotConflict { a: AttachmentId, b: AttachmentId },
}

/// System input metadata and defaults. Runtime assembly belongs to each system
/// factory, not this frontend-facing contract (many systems need no devices).
pub trait InputSystemFactory: InputPorts + std::fmt::Debug {
    fn default_assignments(&self) -> InputAssignments;
}

/// Init-time factory output: field map plus a buffer factory.
/// Endpoint construction belongs to the caller; this stays transport-free.
pub struct InputResources {
    field_map: UnifiedFieldMap,
    new_buffer: BufferFactory,
}

impl std::fmt::Debug for InputResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputResources")
            .field("field_map", &self.field_map)
            .finish_non_exhaustive()
    }
}

impl InputResources {
    pub fn new(field_map: UnifiedFieldMap, new_buffer: BufferFactory) -> Self {
        Self {
            field_map,
            new_buffer,
        }
    }

    pub fn field_map(&self) -> &UnifiedFieldMap {
        &self.field_map
    }

    /// Single move-out for the endpoint constructor: the map plus the
    /// factory travel together, never separately.
    pub fn into_parts(self) -> (UnifiedFieldMap, BufferFactory) {
        (self.field_map, self.new_buffer)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::rc::Rc;

    use super::{
        AbstractKey, ControlInfo, ControlKind, ControllerProfile, CreateSplitError,
        InputAssignments, InputPorts, InputResources, InputSystemFactory, PortSet, SlotInfo,
    };
    use crate::{
        AttachmentId, ControlId, DigitalControlId, InputValue, ProfileId,
        devices::{Port, SimplePort, StrobeTarget},
    };

    #[derive(Debug)]
    struct ProbeProfile;
    impl ControllerProfile for ProbeProfile {
        fn profile_id(&self) -> ProfileId {
            ProfileId::new("test.probe")
        }
        fn label(&self) -> &'static str {
            "Probe"
        }
        fn port_sets(&self) -> &[PortSet] {
            static SETS: &[PortSet] = &[PortSet {
                ports: &[AttachmentId::new("test.p1")],
            }];
            SETS
        }
        fn port_groups(&self) -> &[&[ControlInfo]] {
            const CONTROLS: &[ControlInfo] = &[ControlInfo {
                id: ControlId::digital(DigitalControlId::new("test.a")),
                label: "A",
                kind: ControlKind::Digital,
                abstract_key: Some(AbstractKey::Button1),
            }];
            const GROUPS: &[&[ControlInfo]] = &[CONTROLS];
            GROUPS
        }
    }

    #[derive(Debug)]
    struct ProbePorts;
    impl super::InputPorts for ProbePorts {
        fn slots(&self) -> &[SlotInfo] {
            static SLOTS: &[SlotInfo] = &[SlotInfo {
                id: AttachmentId::new("test.p1"),
                label: "P1",
            }];
            SLOTS
        }
        fn controllers(&self) -> Box<dyn Iterator<Item = Rc<dyn ControllerProfile>> + '_> {
            Box::new(std::iter::once(
                Rc::new(ProbeProfile) as Rc<dyn ControllerProfile>
            ))
        }
    }

    #[derive(Debug)]
    struct ProbeFactory;
    impl super::InputPorts for ProbeFactory {
        fn slots(&self) -> &[SlotInfo] {
            ProbePorts.slots()
        }
        fn controllers(&self) -> Box<dyn Iterator<Item = Rc<dyn ControllerProfile>> + '_> {
            ProbePorts.controllers()
        }
    }
    impl InputSystemFactory for ProbeFactory {
        fn default_assignments(&self) -> InputAssignments {
            InputAssignments::new(vec![(
                AttachmentId::new("test.p1"),
                Some(Rc::new(ProbeProfile) as Rc<dyn ControllerProfile>),
            )])
        }
    }

    #[test]
    fn slot_info_matches_persisted_ids() {
        let slot = SlotInfo {
            id: AttachmentId::new("test.p1"),
            label: "P1",
        };
        assert!(slot.matches_id("test.p1"));
        assert!(!slot.matches_id("test.p2"));
    }

    #[test]
    fn assignments_compare_by_slot_order_and_profile() {
        let profile: Rc<dyn ControllerProfile> = Rc::new(ProbeProfile);
        let first = InputAssignments::new(vec![
            (AttachmentId::new("test.p1"), Some(Rc::clone(&profile))),
            (AttachmentId::new("test.p2"), None),
        ]);
        let same = InputAssignments::new(vec![
            (AttachmentId::new("test.p1"), Some(Rc::clone(&profile))),
            (AttachmentId::new("test.p2"), None),
        ]);
        assert_eq!(first, same);
        let swapped = InputAssignments::new(vec![
            (AttachmentId::new("test.p2"), None),
            (AttachmentId::new("test.p1"), Some(Rc::clone(&profile))),
        ]);
        assert_ne!(first, swapped);
        let dropped = InputAssignments::new(vec![
            (AttachmentId::new("test.p1"), Some(Rc::clone(&profile))),
            (AttachmentId::new("test.p2"), Some(Rc::clone(&profile))),
        ]);
        assert_ne!(first, dropped);
        assert!(format!("{first:?}").contains("test.probe"));
    }

    #[test]
    fn assignments_reject_unknown_multi_port_profile() {
        use super::PortSet;
        #[derive(Debug)]
        struct Multi;
        impl ControllerProfile for Multi {
            fn profile_id(&self) -> ProfileId {
                ProfileId::new("test.multi")
            }
            fn label(&self) -> &'static str {
                "Multi"
            }
            fn port_sets(&self) -> &[PortSet] {
                static SETS: &[PortSet] = &[PortSet {
                    ports: &[AttachmentId::new("test.p1"), AttachmentId::new("test.p2")],
                }];
                SETS
            }
            fn port_groups(&self) -> &[&[ControlInfo]] {
                const EMPTY: &[ControlInfo] = &[];
                const GROUPS: [&[ControlInfo]; 1] = [EMPTY];
                &GROUPS
            }
        }
        let pad: Rc<dyn ControllerProfile> = Rc::new(Multi);
        let assignments = InputAssignments::new(vec![
            (AttachmentId::new("test.p1"), Some(Rc::clone(&pad))),
            (AttachmentId::new("test.p2"), Some(Rc::clone(&pad))),
        ]);
        assert!(matches!(
            assignments.placement_plan(&ProbePorts),
            Err(CreateSplitError::ControllerNotFound { .. })
        ));
    }

    #[test]
    fn resolve_helpers_find_registered_topology() {
        let ports = ProbePorts;
        assert_eq!(
            ports.resolve_slot("test.p1"),
            Some(AttachmentId::new("test.p1"))
        );
        assert_eq!(ports.resolve_slot("missing"), None);
        let found = ports.resolve_controller("test.probe").expect("probe");
        assert_eq!(found.profile_id(), ProfileId::new("test.probe"));
        assert_eq!(found.device_kind_for_group(0).as_str(), "test.probe");
        assert!(ports.resolve_controller("missing").is_none());
    }

    #[test]
    fn profile_reports_default_device_kind() {
        let factory = ProbeFactory;
        let defaults = factory.default_assignments();
        assert_eq!(defaults.slots().len(), 1);
        for err in [
            CreateSplitError::ControllerNotFound {
                controller: ProfileId::new("c"),
            },
            CreateSplitError::IncompatibleController {
                controller: ProfileId::new("c"),
                slot: AttachmentId::new("s"),
            },
            CreateSplitError::SlotConflict {
                a: AttachmentId::new("a"),
                b: AttachmentId::new("b"),
            },
        ] {
            assert!(!err.to_string().is_empty());
        }
    }

    #[test]
    fn resources_map_and_buffer_factory_access() {
        use crate::{BufferError, InputStateBuffer};
        #[derive(Debug, Default)]
        struct Bit;
        impl InputStateBuffer for Bit {
            fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError> {
                match (field, value) {
                    (0, InputValue::Digital(_)) => Ok(()),
                    (0, _) => Err(BufferError::UnsupportedFieldType {
                        field,
                        expected: "d",
                    }),
                    _ => Err(BufferError::FieldNotFound { field }),
                }
            }
            fn clear(&mut self) {}
            fn copy_state(&mut self, other: &dyn InputStateBuffer) -> Result<(), BufferError> {
                if other.downcast_ref::<Self>().is_some() {
                    Ok(())
                } else {
                    Err(BufferError::MismatchedBuffer {
                        expected: "Bit",
                        actual: "?",
                    })
                }
            }
        }
        let resources = InputResources::new(
            HashMap::from([(
                (
                    AttachmentId::new("test.p1"),
                    ControlId::digital(DigitalControlId::new("test.a")),
                ),
                0,
            )]),
            Box::new(|| Box::new(Bit) as Box<dyn InputStateBuffer>),
        );
        assert_eq!(resources.field_map().len(), 1);
        // Single move-out: the map plus the factory travel together.
        let (map, factory) = resources.into_parts();
        assert_eq!(map.len(), 1);
        let mut buffer = factory();
        assert!(buffer.downcast_ref::<Bit>().is_some());
        assert!(buffer.set(0, InputValue::Digital(true)).is_ok());
        assert!(buffer.set(9, InputValue::Digital(true)).is_err());
        let port = SimplePort::new(0, "test.p1");
        let _ = (&port as &dyn Port).port_id();
        let mut target = ProbePadTarget;
        StrobeTarget::strobe(&mut target, 1);
    }

    #[derive(Debug)]
    struct ProbePadTarget;
    impl StrobeTarget for ProbePadTarget {
        fn strobe(&mut self, _value: u8) {}
    }
}
