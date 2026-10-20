//! Controller profiles, slot assignments, and the input factory plan.

use std::{collections::HashMap, rc::Rc};

use super::{
    devices::ControllerCollection,
    events::{InputStateBuffer, InputValue},
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

    /// Assign a controller to a known slot; unknown slots refuse instead of
    /// growing the topology behind the factory's back.
    pub fn set_assignment(
        &mut self,
        slot: AttachmentId,
        controller: Option<Rc<dyn ControllerProfile>>,
    ) -> Result<(), CreateSplitError> {
        let Some(entry) = self.slots.iter_mut().find(|(id, _)| *id == slot) else {
            return Err(CreateSplitError::SlotNotFound { slot });
        };
        entry.1 = controller;
        Ok(())
    }

    /// Clear every other slot sharing a multi-port set with `slot`.
    /// Single owner for slot-topology mutation; settings layers delegate here.
    pub fn clear_multi_port_conflicts(
        &mut self,
        slot: AttachmentId,
        profile: &dyn ControllerProfile,
    ) {
        for ps in profile.port_sets() {
            if ps.ports.len() <= 1 || !ps.ports.contains(&slot) {
                continue;
            }
            for &port in ps.ports {
                if port != slot
                    && let Some(other) = self.slots.iter_mut().find(|(s, _)| *s == port)
                {
                    other.1 = None;
                }
            }
        }
    }

    /// Explicit order-sensitive structural comparison used by `PartialEq`.
    pub fn eq_ordered(&self, other: &Self) -> bool {
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
        self.eq_ordered(other)
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

/// Factory for creating input runtime state.
pub trait InputSystemFactory: InputPorts + std::fmt::Debug {
    fn default_assignments(&self) -> InputAssignments;
    fn create_split(
        &self,
        controllers: &ControllerCollection,
    ) -> Result<InputResources, CreateSplitError>;
}

/// Init-time factory output: field map plus a buffer factory.
/// Thread endpoints live behind `nerust_emu_thread::InputPort`; this stays thread-free.
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

    /// Replace the field map with revalidation: field indices must be
    /// unique, so two controls can never alias one absolute field.
    pub fn replace_field_map(
        &mut self,
        next: HashMap<(AttachmentId, ControlId), usize>,
    ) -> Result<(), CreateSplitError> {
        let mut owner = std::collections::HashMap::with_capacity(next.len());
        for ((attachment, _), field) in &next {
            if let Some(first) = owner.insert(*field, *attachment) {
                return Err(CreateSplitError::SlotConflict {
                    a: first,
                    b: *attachment,
                });
            }
        }
        self.field_map = next;
        Ok(())
    }

    /// Single move-out for the endpoint constructor: the map plus the
    /// factory travel together into `InputSplit`, never separately.
    pub fn into_parts(self) -> (UnifiedFieldMap, BufferFactory) {
        (self.field_map, self.new_buffer)
    }

    /// Write one absolute field through a fresh buffer (init-time helper).
    pub fn probe_set(
        &self,
        field: usize,
        value: InputValue,
    ) -> Result<(), super::events::BufferError> {
        let mut buf = (self.new_buffer)();
        buf.set(field, value)
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
        AttachmentId, ControlId, ControllerCollection, DigitalControlId, InputValue, ProfileId,
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
        fn create_split(
            &self,
            _controllers: &ControllerCollection,
        ) -> Result<InputResources, CreateSplitError> {
            Err(CreateSplitError::SlotNotFound {
                slot: AttachmentId::new("test.p1"),
            })
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
        assert!(first.eq_ordered(&same));
        assert_eq!(first, same);
        let swapped = InputAssignments::new(vec![
            (AttachmentId::new("test.p2"), None),
            (AttachmentId::new("test.p1"), Some(Rc::clone(&profile))),
        ]);
        assert!(!first.eq_ordered(&swapped));
        assert_ne!(first, swapped);
        let dropped = InputAssignments::new(vec![
            (AttachmentId::new("test.p1"), Some(Rc::clone(&profile))),
            (AttachmentId::new("test.p2"), Some(Rc::clone(&profile))),
        ]);
        assert!(!first.eq_ordered(&dropped));
        let mut mutable = first.clone();
        mutable
            .set_assignment(AttachmentId::new("test.p2"), Some(Rc::clone(&profile)))
            .unwrap();
        assert_eq!(mutable.slots().len(), 2);
        assert!(format!("{mutable:?}").contains("test.probe"));
        // Unknown slots refuse instead of growing the topology.
        assert!(
            mutable
                .set_assignment(AttachmentId::new("test.missing"), None)
                .is_err()
        );
    }

    #[test]
    fn assignments_own_multi_port_clearing() {
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
        let mut assignments = InputAssignments::new(vec![
            (AttachmentId::new("test.p1"), Some(Rc::clone(&pad))),
            (AttachmentId::new("test.p2"), Some(Rc::clone(&pad))),
        ]);
        assignments.clear_multi_port_conflicts(AttachmentId::new("test.p1"), pad.as_ref());
        assert!(assignments.slots()[0].1.is_some());
        assert!(assignments.slots()[1].1.is_none());
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
        let err = factory.create_split(&ControllerCollection::new(vec![]));
        assert!(matches!(err, Err(CreateSplitError::SlotNotFound { .. })));
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
    fn resources_probe_set_and_map_access() {
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
        let mut resources = InputResources::new(
            HashMap::from([(
                (
                    AttachmentId::new("test.p1"),
                    ControlId::digital(DigitalControlId::new("test.a")),
                ),
                0,
            )]),
            Box::new(|| Box::new(Bit) as Box<dyn InputStateBuffer>),
        );
        assert!(resources.probe_set(0, InputValue::Digital(true)).is_ok());
        assert!(resources.probe_set(9, InputValue::Digital(true)).is_err());
        assert_eq!(resources.field_map().len(), 1);
        // Revalidation refuses aliased fields; disjoint maps replace cleanly.
        let mut dup = HashMap::from([
            (
                (
                    AttachmentId::new("test.p1"),
                    ControlId::digital(DigitalControlId::new("test.a")),
                ),
                0,
            ),
            (
                (
                    AttachmentId::new("test.p1"),
                    ControlId::digital(DigitalControlId::new("test.b")),
                ),
                0,
            ),
        ]);
        assert!(resources.replace_field_map(dup.clone()).is_err());
        dup.remove(&(
            AttachmentId::new("test.p1"),
            ControlId::digital(DigitalControlId::new("test.b")),
        ));
        resources.replace_field_map(dup).unwrap();
        assert_eq!(resources.field_map().len(), 1);
        // Single move-out: the map plus the factory travel together.
        let (map, factory) = resources.into_parts();
        assert_eq!(map.len(), 1);
        assert!((factory)().downcast_ref::<Bit>().is_some());
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
