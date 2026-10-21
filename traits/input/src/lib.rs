//! Input vocabulary shared by cores, factories, and frontends.
//! Split by responsibility: `ids`, `topology`, `events`, `devices`, `profile`.

mod devices;
mod events;
mod ids;
mod profile;
mod topology;

pub use devices::{
    Controller, ControllerCollection, ControllerHub, FieldEntry, Port, SimplePort, StrobeTarget,
};
pub use events::{
    BufferError, DigitalInputEvent, DigitalInputState, FieldId, InputEvent, InputReceiver,
    InputStateBuffer, InputValue, OpenBusReadResult,
};
pub use ids::{
    AnalogControlId, AttachmentId, ControlId, DeviceKindId, DigitalControlId, PortId, ProfileId,
};
pub use profile::{
    AbstractKey, BufferFactory, ControlInfo, ControlKind, ControllerProfile, CreateSplitError,
    InputAssignments, InputPorts, InputResources, InputSystemFactory, PortSet, SlotInfo,
    UnifiedFieldMap,
};
pub use topology::{
    AnalogControlDescriptor, AttachmentSlotDescriptor, ControlDescriptor, DeviceDescriptor,
    DigitalControlDescriptor, InputTopologyDescriptor, PortDescriptor,
};
