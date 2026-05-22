pub use nerust_console::ConsoleMetrics;
pub use nerust_gui_session::{
    ButtonDescriptor, ConsoleError, ControllerDescriptor, ControllerInput, ControllerPort,
    InputState, SessionCommand, SessionCommandOutcome, SessionCore, window_title,
};
pub use nerust_persistence::StateSlotSummary;
pub use nerust_screen_filter::ConsoleVideoAssets;
pub use nerust_screen_traits::{PhysicalSize, VideoPresentation};

mod session;
mod slots;

pub use self::session::{ConsoleSessionFactory, GuiSession};
pub use self::slots::slot_label;
