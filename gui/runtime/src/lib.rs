pub use nerust_console::ConsoleMetrics;
pub use nerust_gui_session::{
    ButtonDescriptor, ConsoleError, ConsoleVideo, ControllerDescriptor, ControllerInput,
    ControllerPort, InputState, SessionCommand, SessionCommandOutcome, SessionCore, WindowSize,
    window_title,
};
pub use nerust_persistence::StateSlotSummary;

pub mod options;
mod session;
mod slots;

pub use self::session::{ConsoleSessionFactory, GuiSession};
pub use self::slots::slot_label;
