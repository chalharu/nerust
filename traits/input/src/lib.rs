//! Input vocabulary shared by cores, factories, and frontends.
//! Split by responsibility: `ids`, `topology`, `events`, `devices`, `profile`.

mod backend;
mod devices;
mod events;
mod ids;
mod key;
mod profile;
mod source;
mod topology;

pub use backend::{
    Capability, DeviceEvent, FakeSink, FakeSource, PollSource, RumbleSink,
};
pub use devices::{
    BUILTIN_KEYBOARD_MEMBER, Controller, ControllerCollection, ControllerHub, FieldEntry,
    GAMEPAD_MEMBER_PREFIX, KEYBOARD_MEMBER_PREFIX, Port, SimplePort, StrobeTarget,
    member_is_gamepad, member_is_shareable,
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
    TypedInputResources, UnifiedFieldMap, abstract_for_attachment, collect_first_wins,
    port_index_for_slot,
};
pub use source::{
    GamepadAxis, GamepadEvent, GamepadMotion, PadGuid, SourceButtonId, SourceId, StandardButton,
    apply_deadzone,
};
pub use topology::{
    AnalogControlDescriptor, AttachmentSlotDescriptor, ControlDescriptor, DeviceDescriptor,
    DigitalControlDescriptor, InputTopologyDescriptor, PortDescriptor,
};
