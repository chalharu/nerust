//! Debugger panel: disassembly text plus Pause/Resume/Step.
//!
//! A plain `NSTextView` renders drained `DebuggerModel` rows; stepping
//! goes through the shared shell drain exactly like the Win32 twin.

use nerust_frontend_host::debugger::DebuggerModel;
use nerust_gui_shell::{
    debug::{DebugAction, DebugDrainInput, DrainFlight, DrainPoll},
    session::commands::SessionCommand,
};
use objc2::{MainThreadMarker, rc::Retained, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSTextField, NSTextView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::app::anyobject;

pub struct DbgWin {
    pub window: Retained<NSWindow>,
    text: Retained<NSTextView>,
    status: Retained<NSTextField>,
    model: DebuggerModel,
    flight: Option<DrainFlight>,
    prev_rows: Vec<nerust_core_traits::debugger::HexRow>,
}

impl DbgWin {
    pub fn open(
        mtm: MainThreadMarker,
        host: &mut nerust_mac::MacHost,
        delegate: &super::app::AppDelegate,
    ) -> Option<Self> {
        let frame = NSRect::new(NSPoint::new(760.0, 100.0), NSSize::new(560.0, 420.0));
        let window: Retained<NSWindow> = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc::<NSWindow>(),
                frame,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&NSString::from_str("Debugger"));
        let text: Retained<NSTextView> = NSTextView::initWithFrame(
            mtm.alloc::<NSTextView>(),
            NSRect::new(NSPoint::new(8.0, 60.0), NSSize::new(544.0, 320.0)),
        );
        text.setEditable(false);
        window.setContentView(Some(&text));
        let button = |x: f64, title: &str, action: objc2::runtime::Sel| {
            let btn: Retained<NSButton> = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(title),
                    Some(anyobject(delegate)),
                    Some(action),
                    mtm,
                )
            };
            btn.setFrame(NSRect::new(NSPoint::new(x, 22.0), NSSize::new(90.0, 24.0)));
            if let Some(view) = window.contentView() {
                view.addSubview(&btn);
            }
        };
        button(8.0, "Pause", sel!(debugPause:));
        button(104.0, "Resume", sel!(debugResume:));
        button(200.0, "Step", sel!(debugStep:));
        let status: Retained<NSTextField> = NSTextField::initWithFrame(
            mtm.alloc::<NSTextField>(),
            NSRect::new(NSPoint::new(8.0, 8.0), NSSize::new(544.0, 20.0)),
        );
        if let Some(view) = window.contentView() {
            view.addSubview(&status);
        }
        window.makeKeyAndOrderFront(None);
        let win = Self {
            window,
            text,
            status,
            model: DebuggerModel::from_session(host.session()),
            flight: None,
            prev_rows: Vec::new(),
        };
        win.refresh();
        Some(win)
    }

    pub fn on_tick(&mut self, host: &mut nerust_mac::MacHost) {
        if let Some(flight) = self.flight.as_mut() {
            match flight.poll() {
                DrainPoll::Pending => {}
                DrainPoll::Done(output) => {
                    self.model.apply_drain(output.display, output.reselected);
                    self.prev_rows = self.model.display.dump_rows.clone();
                    self.refresh();
                    self.flight = None;
                }
            }
            return;
        }
        let space = host
            .session()
            .debug_spaces()
            .into_iter()
            .next()
            .map(|info| info.id);
        let input = DebugDrainInput {
            space,
            mem_addr: 0,
            dis_addr: None,
            watch: None,
            freeze: None,
            reg_target: None,
            prev_dump_rows: &self.prev_rows,
            need_images: false,
        };
        self.flight = Some(
            host.session()
                .debug_drain_begin(&[DebugAction::Refresh], &input),
        );
    }

    pub fn step(&mut self, host: &mut nerust_mac::MacHost) {
        let _ = host.dispatch_cmd(SessionCommand::Pause);
        let space = host
            .session()
            .debug_spaces()
            .into_iter()
            .next()
            .map(|info| info.id);
        let input = DebugDrainInput {
            space,
            mem_addr: 0,
            dis_addr: None,
            watch: None,
            freeze: None,
            reg_target: None,
            prev_dump_rows: &self.prev_rows,
            need_images: false,
        };
        self.flight = Some(
            host.session()
                .debug_drain_begin(&[DebugAction::StepInstr], &input),
        );
    }

    fn refresh(&self) {
        let mut text = String::new();
        for line in self.model.display.disasm_lines.iter().take(200) {
            let bytes: String = line.bytes[..line.len as usize]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let mark = if line.is_pc { ">" } else { " " };
            text.push_str(&format!(
                "{mark}{:08X}: {:<12} {}\n",
                line.addr, bytes, line.text
            ));
        }
        self.text.setString(&NSString::from_str(&text));
        let state = if self.model.display.paused {
            "PAUSED"
        } else {
            "RUNNING"
        };
        self.status.setStringValue(&NSString::from_str(&format!(
            "{state} | {} disasm lines",
            self.model.display.disasm_lines.len()
        )));
    }
}
