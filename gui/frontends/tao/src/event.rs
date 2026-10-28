//! Host events transported unchanged through Tao's native event loop.

pub(crate) use nerust_frontend_host::commands::MenuCommand;
pub(crate) use nerust_frontend_viewdefs::event::EventCommand as UserEvent;
use nerust_frontend_viewdefs::event::EventSink;
use tao::event_loop::EventLoopProxy;

pub(crate) struct TaoEventSink(pub(crate) EventLoopProxy<UserEvent>);

impl EventSink for TaoEventSink {
    fn send(&self, event: UserEvent) -> bool {
        self.0.send_event(event).is_ok()
    }
}
