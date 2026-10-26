//! PPU viewer: live pattern-table images plus observation panels.
//!
//! A 500ms-throttled Refresh drain with `need_images` feeds both pictures
//! and the formatted panels, mirroring the tao twin. Hover readout polls
//! the mouse location on the tick (no tracking-area subclass needed).

use std::time::Instant;

use nerust_frontend_host::debugger::{DebuggerEvent, DebuggerModel};
use nerust_gui_shell::debug::{DebugAction, DebugDrainInput, DrainFlight, DrainPoll};
use objc2::{AnyThread, MainThreadMarker, rc::Retained};
use objc2_app_kit::NSCalibratedRGBColorSpace;
use objc2_app_kit::{
    NSBackingStoreType, NSBitmapFormat, NSBitmapImageRep, NSImage, NSImageView, NSTextField,
    NSTextView, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};

use super::app::rgba_to_bgrx;

pub struct PpuWin {
    pub window: Retained<NSWindow>,
    model: DebuggerModel,
    flight: Option<DrainFlight>,
    last_poll: Instant,
    hover: Retained<NSTextField>,
    panels: Retained<NSTextView>,
    stack: Retained<NSView>,
    views: Vec<(Retained<NSImageView>, u32, u32)>,
    mtm: MainThreadMarker,
    reps: Vec<Retained<NSBitmapImageRep>>,
}

fn make_rep(mtm: MainThreadMarker, w: u32, h: u32) -> Option<Retained<NSBitmapImageRep>> {
    unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            mtm.alloc::<NSBitmapImageRep>(),
            std::ptr::null_mut(),
            w as isize,
            h as isize,
            8,
            4,
            true,
            false,
            NSCalibratedRGBColorSpace,
            NSBitmapFormat::AlphaFirst | NSBitmapFormat::ThirtyTwoBitLittleEndian,
            (w * 4) as isize,
            32,
        )
    }
}

impl PpuWin {
    pub fn open(mtm: MainThreadMarker, host: &mut nerust_mac::MacHost) -> Option<Self> {
        let frame = NSRect::new(NSPoint::new(760.0, 60.0), NSSize::new(340.0, 640.0));
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
        window.setTitle(&NSString::from_str("PPU viewer"));
        let stack: Retained<NSView> = NSView::initWithFrame(
            mtm.alloc::<NSView>(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(340.0, 640.0)),
        );
        let hover: Retained<NSTextField> = NSTextField::initWithFrame(
            mtm.alloc::<NSTextField>(),
            NSRect::new(NSPoint::new(8.0, 610.0), NSSize::new(324.0, 20.0)),
        );
        hover.setEditable(false);
        hover.setBezeled(false);
        hover.setDrawsBackground(false);
        stack.addSubview(&hover);
        let panels: Retained<NSTextView> = NSTextView::initWithFrame(
            mtm.alloc::<NSTextView>(),
            NSRect::new(NSPoint::new(8.0, 8.0), NSSize::new(324.0, 120.0)),
        );
        panels.setEditable(false);
        stack.addSubview(&panels);
        window.setContentView(Some(&stack));
        window.makeKeyAndOrderFront(None);
        let mut win = Self {
            window,
            model: DebuggerModel::from_session(host.session()),
            flight: None,
            last_poll: Instant::now()
                .checked_sub(std::time::Duration::from_secs(1))
                .unwrap_or_else(Instant::now),
            hover,
            panels,
            stack,
            views: Vec::new(),
            mtm,
            reps: Vec::new(),
        };
        win.on_tick(host);
        Some(win)
    }

    pub fn on_tick(&mut self, host: &mut nerust_mac::MacHost) {
        if let Some(flight) = self.flight.as_mut() {
            match flight.poll() {
                DrainPoll::Pending => {}
                DrainPoll::Done(output) => {
                    self.model.apply_drain(output.display, output.reselected);
                    self.rebuild();
                    self.flight = None;
                }
            }
        } else if self.last_poll.elapsed().as_millis() >= 500 {
            self.last_poll = Instant::now();
            let _ = self.model.dispatch(DebuggerEvent::RefreshPressed);
            let prev = self.model.display.dump_rows.clone();
            let input = DebugDrainInput {
                space: self.model.selected_space(),
                mem_addr: self.model.anchor(),
                dis_addr: self.model.nav.dis_addr(),
                watch: None,
                freeze: None,
                reg_target: None,
                prev_dump_rows: &prev,
                need_images: true,
            };
            let _ = &prev;
            self.flight = Some(
                host.session()
                    .debug_drain_begin(&[DebugAction::Refresh], &input),
            );
        }
        self.poll_hover();
    }

    fn poll_hover(&mut self) {
        let mouse = self.window.mouseLocationOutsideOfEventStream();
        for (view, w, h) in &self.views {
            let at = view.convertPoint_fromView(mouse, None);
            let (w2, h2) = (f64::from(*w * 2), f64::from(*h * 2));
            if at.x >= 0.0 && at.x <= w2 && at.y >= 0.0 && at.y <= h2 {
                let cols = w / 8;
                let tx = (at.x / 16.0) as u32;
                let ty = ((h2 - at.y) / 16.0) as u32;
                self.hover.setStringValue(&NSString::from_str(&format!(
                    "tile #{} ({},{} +{})",
                    ty * cols + tx,
                    tx * 8,
                    ty * 8,
                    (ty * cols + tx) as usize * 16
                )));
                return;
            }
        }
    }

    fn rebuild(&mut self) {
        // Drop old image views (keep hover label + panels text).
        self.reps.clear();
        self.views.clear();
        let current = self.stack.subviews();
        let mut subs = Vec::new();
        for index in 0..current.count() {
            let view: Retained<NSView> = current.objectAtIndex(index);
            if Retained::as_ptr(&view) == Retained::as_ptr(&self.hover) as *const NSView
                || Retained::as_ptr(&view) == Retained::as_ptr(&self.panels) as *const NSView
            {
                subs.push(view);
            }
        }
        self.stack.setSubviews(&NSArray::from_retained_slice(&[]));
        for view in subs {
            self.stack.addSubview(&view);
        }
        let mut y = 600.0;
        for image in self.model.display.images.clone() {
            let (w, h) = (image.width, image.height);
            let Some(rep) = make_rep(self.mtm, w, h) else {
                continue;
            };
            let bgrx = rgba_to_bgrx(w, h, &image.rgba);
            unsafe {
                let dst = rep.bitmapData();
                std::ptr::copy_nonoverlapping(
                    bgrx.as_ptr(),
                    dst,
                    bgrx.len().min((w * h * 4) as usize),
                );
            }
            let picture = NSImage::initWithSize(
                NSImage::alloc(),
                NSSize::new(f64::from(w * 2), f64::from(h * 2)),
            );
            picture.addRepresentation(&rep);
            self.reps.push(rep);
            let view: Retained<NSImageView> = NSImageView::initWithFrame(
                self.mtm.alloc::<NSImageView>(),
                NSRect::new(
                    NSPoint::new(8.0, y - f64::from(h * 2) - 20.0),
                    NSSize::new(f64::from(w * 2), f64::from(h * 2)),
                ),
            );
            view.setImage(Some(&picture));
            let title: Retained<NSTextField> = NSTextField::initWithFrame(
                self.mtm.alloc::<NSTextField>(),
                NSRect::new(NSPoint::new(8.0, y - 18.0), NSSize::new(324.0, 18.0)),
            );
            title.setStringValue(&NSString::from_str(&image.label));
            title.setEditable(false);
            title.setBezeled(false);
            title.setDrawsBackground(false);
            self.stack.addSubview(&title);
            self.stack.addSubview(&view);
            self.views.push((view, w, h));
            y -= f64::from(h * 2) + 24.0;
        }
        let panels = nerust_gui_viewmodel::debugger::format_panels(&self.model.display.panels);
        self.panels.setString(&NSString::from_str(&panels));
    }
}
