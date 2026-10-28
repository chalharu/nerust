//! Host events transported unchanged through Tao's native event loop.

use tao::event_loop::EventLoopProxy;
pub(crate) use wakui::commands::MenuCommand;
pub(crate) use wakui::event::EventCommand as UserEvent;
use wakui::event::EventSink;

pub(crate) struct TaoEventSink(pub(crate) EventLoopProxy<UserEvent>);

impl EventSink for TaoEventSink {
    fn send(&self, event: UserEvent) -> bool {
        self.0.send_event(event).is_ok()
    }
}
