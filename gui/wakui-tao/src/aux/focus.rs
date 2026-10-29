//! Aux-window focus agreement as behavior-carrying types.
//! Aux handles implement [`AuxWindow`] + [`AuxEvents`]; runtime only drives them.

use crate::aux::shell::AuxShellOps;
use crate::domain::{AuxNativeEvent, NativeWindowId};

/// Identity half: routing, liveness, and redraw requests.
pub(crate) trait AuxIdentity {
    /// Native window identity for event routing.
    fn native_id(&self) -> NativeWindowId;
    /// Close was requested through the shared flag.
    fn close_requested(&self) -> bool;
    /// Bring this viewer to the front.
    fn focus_window(&self);
    /// Native redraw request without reaching into the window.
    fn request_redraw(&self);
}

/// Render half: event prelude, conversion, and full render.
pub(crate) trait AuxRender {
    /// Shared event prelude: resize/scale/modifiers/close.
    fn handle_common(&mut self, event: &AuxNativeEvent);
    /// Convert one domain event for the window's own dispatcher.
    fn feed_event(&mut self, event: AuxNativeEvent) -> Option<iced::Event>;
    /// Full render through the shared pump; the title tags the log lines.
    fn render(&mut self, title: &str) -> Option<String>;
    fn wake_deadline(&self) -> Option<std::time::Instant> {
        None
    }
    fn wake_due(&mut self, _now: std::time::Instant) {}
}

/// Combined pump surface: every aux window speaks both halves.
pub(crate) trait AuxWindow: AuxIdentity + AuxRender {}
impl<T: AuxIdentity + AuxRender + ?Sized> AuxWindow for T {}

/// Single shell accessor behind every aux handle.
/// Blanket [`AuxIdentity`]/[`AuxRender`] impls delegate here.
pub(crate) trait AuxShellRef {
    fn shell(&self) -> &dyn AuxShellOps;
    fn shell_mut(&mut self) -> &mut dyn AuxShellOps;
}

impl<T: AuxShellRef + ?Sized> AuxIdentity for T {
    fn native_id(&self) -> NativeWindowId {
        self.shell().native_id()
    }

    fn close_requested(&self) -> bool {
        self.shell().close_requested()
    }

    fn focus_window(&self) {
        self.shell().focus_window();
    }

    fn request_redraw(&self) {
        self.shell().request_redraw();
    }
}

impl<T: AuxShellRef + ?Sized> AuxRender for T {
    fn handle_common(&mut self, event: &AuxNativeEvent) {
        self.shell_mut().handle_common(event);
    }

    fn feed_event(&mut self, event: AuxNativeEvent) -> Option<iced::Event> {
        self.shell_mut().feed_event(event)
    }

    fn render(&mut self, title: &str) -> Option<String> {
        self.shell_mut().render_tagged(title)
    }

    fn wake_deadline(&self) -> Option<std::time::Instant> {
        self.shell().wake_deadline()
    }
    fn wake_due(&mut self, now: std::time::Instant) {
        self.shell_mut().wake_due(now);
    }
}

/// Uniform aux-window event handling: one converted event in.
/// Returns whether this pass already requested a redraw.
pub(crate) trait AuxEvents {
    fn handle(&mut self, mapped: iced::Event) -> bool;
}

/// Feed one domain event through the shared prelude + program dispatch + redraw.
pub(crate) fn pump_aux<W: AuxWindow + AuxEvents + ?Sized>(window: &mut W, event: AuxNativeEvent) {
    window.handle_common(&event);
    match window.feed_event(event) {
        Some(mapped) => {
            if !window.handle(mapped) {
                window.request_redraw();
            }
        }
        None => window.request_redraw(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::{Cell, RefCell};

    /// Headless shell double: records redraw/focus, feeds canned events.
    struct FakeShell {
        nid: NativeWindowId,
        close: bool,
        mapped: Option<iced::Event>,
        redraws: Rc<Cell<u32>>,
        focused: Rc<Cell<bool>>,
        seen_tag: Rc<RefCell<Option<String>>>,
    }

    use std::rc::Rc;

    impl crate::aux::shell::AuxShellOps for FakeShell {
        fn native_id(&self) -> NativeWindowId {
            self.nid
        }
        fn close_requested(&self) -> bool {
            self.close
        }
        fn focus_window(&self) {
            self.focused.set(true);
        }
        fn request_redraw(&self) {
            self.redraws.set(self.redraws.get() + 1);
        }
        fn handle_common(&mut self, _event: &AuxNativeEvent) {}
        fn feed_event(&mut self, _event: AuxNativeEvent) -> Option<iced::Event> {
            self.mapped.take()
        }
        fn render_tagged(&mut self, tag: &str) -> Option<String> {
            *self.seen_tag.borrow_mut() = Some(tag.to_string());
            None
        }
    }

    /// Window double behind the blanket `AuxIdentity`/`AuxRender` impls.
    struct FakeWin {
        shell: FakeShell,
        handled: Rc<Cell<bool>>,
        handle_ret: bool,
    }

    impl AuxShellRef for FakeWin {
        fn shell(&self) -> &dyn crate::aux::shell::AuxShellOps {
            &self.shell
        }
        fn shell_mut(&mut self) -> &mut dyn crate::aux::shell::AuxShellOps {
            &mut self.shell
        }
    }

    impl AuxEvents for FakeWin {
        fn handle(&mut self, _mapped: iced::Event) -> bool {
            self.handled.set(true);
            self.handle_ret
        }
    }

    fn fake_win(mapped: Option<iced::Event>, handle_ret: bool) -> FakeWin {
        FakeWin {
            shell: FakeShell {
                nid: NativeWindowId::dummy(),
                close: false,
                mapped,
                redraws: Rc::new(Cell::new(0)),
                focused: Rc::new(Cell::new(false)),
                seen_tag: Rc::new(RefCell::new(None)),
            },
            handled: Rc::new(Cell::new(false)),
            handle_ret,
        }
    }

    fn redraw_event() -> iced::Event {
        iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ))
    }

    #[test]
    fn pump_aux_redraws_only_when_unhandled() {
        // Handled event: no redraw requested.
        let mut win = fake_win(Some(redraw_event()), true);
        pump_aux(&mut win, AuxNativeEvent::Focused(true));
        assert!(win.handled.get());
        assert_eq!(win.shell.redraws.get(), 0);
        // Unhandled event: host redraws through the shell.
        let mut win = fake_win(Some(redraw_event()), false);
        pump_aux(&mut win, AuxNativeEvent::Focused(true));
        assert!(win.handled.get());
        assert_eq!(win.shell.redraws.get(), 1);
        // No mapped event: redraw without dispatch.
        let mut win = fake_win(None, true);
        pump_aux(&mut win, AuxNativeEvent::Focused(true));
        assert!(!win.handled.get());
        assert_eq!(win.shell.redraws.get(), 1);
    }

    #[test]
    fn blanket_impls_delegate_identity_and_render() {
        let mut win = fake_win(Some(redraw_event()), true);
        assert_eq!(AuxIdentity::native_id(&win), win.shell.nid);
        assert!(!AuxIdentity::close_requested(&win));
        AuxIdentity::focus_window(&win);
        assert!(win.shell.focused.get());
        AuxIdentity::request_redraw(&win);
        assert_eq!(win.shell.redraws.get(), 1);
        // Render fans out with the window title as tag.
        AuxRender::render(&mut win, "Aux Alpha");
        assert_eq!(*win.shell.seen_tag.borrow(), Some("Aux Alpha".to_string()));
        win.shell.close = true;
        assert!(AuxIdentity::close_requested(&win));
    }
}
