//! Input vocabulary shared by cores, factories, and frontends.
//! Split by responsibility: `ids`, `topology`, `events`, `devices`, `profile`.

mod devices;
mod events;
mod ids;
mod key;
mod profile;
mod source;
mod topology;

pub use devices::{
    Controller, ControllerCollection, ControllerHub, FieldEntry, Port, SimplePort, StrobeTarget,
};
pub use events::{
    BufferError, DigitalInputEvent, DigitalInputState, GuiInputPort, InputEvent, InputPublish,
    InputReceiver, InputStaging, InputStateBuffer, InputValue, OpenBusReadResult,
};
pub use ids::{
    AnalogControlId, AttachmentId, ControlId, DeviceKindId, DigitalControlId, PortId, ProfileId,
};
pub use key::Key;
pub use profile::{
    AbstractKey, BufferFactory, ControlInfo, ControlKind, ControllerProfile, CreateSplitError,
    InputAssignments, InputPorts, InputResources, InputSystemFactory, PortSet, SlotInfo,
    TypedInputResources, UnifiedFieldMap, collect_first_wins,
};
pub use source::{
    GamepadAxis, GamepadState, MAX_PADS, PadGuid, SourceButtonId, SourceEvent, SourceId,
    StandardButton, apply_deadzone,
};
pub use topology::{
    AnalogControlDescriptor, AttachmentSlotDescriptor, ControlDescriptor, DeviceDescriptor,
    DigitalControlDescriptor, InputTopologyDescriptor, PortDescriptor,
};
