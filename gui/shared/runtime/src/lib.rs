pub use self::console_api::{ConsoleMetrics, ControllerInputs};
pub use self::session_api::{
    ButtonDescriptor, ConsoleError, ConsoleVideo, ControllerDescriptor, ControllerInput,
    ControllerPort, InputState, SessionCommand, SessionCommandOutcome, SessionCore, WindowSize,
    window_title,
};
pub use nerust_persistence::StateSlotSummary;

pub mod console_api;
pub mod options;
mod session;
mod session_api;
mod slots;

pub use self::session::{ConsoleSessionFactory, GuiSession};
pub use self::slots::slot_label;
