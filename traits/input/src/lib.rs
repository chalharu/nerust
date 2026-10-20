//! Input vocabulary shared by cores, factories, and frontends.
//! Split by responsibility: `ids`, `topology`, `events`, `devices`, `profile`.

mod devices;
mod events;
mod ids;
mod profile;
mod topology;

pub use devices::{Controller, ControllerCollection, ControllerHub, FieldEntry, Port, SimplePort};
pub use events::{
    BufferError, DigitalInputEvent, DigitalInputState, InputEvent, InputStateBuffer, InputValue,
    OpenBusReadResult,
};
pub use ids::{AnalogControlId, AttachmentId, DeviceKindId, DigitalControlId, PortId, ProfileId};
pub use profile::{
    AbstractKey, ControlInfo, ControlKind, ControllerProfile, CreateSplitError, EmuInput, GuiInput,
    InputAssignments, InputPorts, InputResources, InputSplit, InputSystemFactory, PortSet,
    SlotInfo,
};
pub use topology::{
    AnalogControlDescriptor, AttachmentSlotDescriptor, ControlDescriptor, ControlId,
    DeviceDescriptor, DigitalControlDescriptor, InputTopologyDescriptor, PortDescriptor,
};
