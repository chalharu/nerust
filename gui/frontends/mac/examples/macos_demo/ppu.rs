//! PPU viewer window: live pattern-table image via the shared live flight.
//!
//! An `NSImageView` shows the first live image; the 500ms display-link
//! tick polls the in-flight fetch, mirroring the Win32 twin.

use objc2::{AnyThread, MainThreadMarker, rc::Retained};
use objc2_app_kit::NSCalibratedRGBColorSpace;
use objc2_app_kit::{
    NSBackingStoreType, NSBitmapFormat, NSBitmapImageRep, NSImage, NSImageView, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::app::rgba_to_bgrx;

pub struct PpuWin {
    pub window: Retained<NSWindow>,
    image_view: Retained<NSImageView>,
    flight: Option<nerust_gui_shell::debug::LiveImagesFlight>,
    generation: u64,
    picture: Option<PpuPicture>,
    mtm: MainThreadMarker,
}

struct PpuPicture {
    width: u32,
    height: u32,
    bgrx: Vec<u8>,
}

impl PpuWin {
    pub fn open(mtm: MainThreadMarker, _host: &mut nerust_mac::MacHost) -> Option<Self> {
        let frame = NSRect::new(NSPoint::new(760.0, 100.0), NSSize::new(300.0, 320.0));
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
        let image_view: Retained<NSImageView> = NSImageView::initWithFrame(
            mtm.alloc::<NSImageView>(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(300.0, 320.0)),
        );
        window.setContentView(Some(&image_view));
        window.makeKeyAndOrderFront(None);
        Some(Self {
            window,
            image_view,
            flight: None,
            generation: 0,
            picture: None,
            mtm,
        })
    }

    pub fn on_tick(&mut self, host: &mut nerust_mac::MacHost) {
        use nerust_gui_shell::debug::LiveImagesPoll;
        if let Some(flight) = self.flight.as_mut() {
            match flight.poll() {
                LiveImagesPoll::Pending => {}
                LiveImagesPoll::Done(batch) => {
                    if let Some(batch) = batch
                        && batch.generation != self.generation
                        && let Some(image) = batch.images.into_iter().next()
                    {
                        self.generation = batch.generation;
                        self.window.setTitle(&NSString::from_str(&format!(
                            "PPU viewer — {}",
                            image.label
                        )));
                        self.picture = Some(PpuPicture {
                            width: image.width,
                            height: image.height,
                            bgrx: rgba_to_bgrx(image.width, image.height, &image.rgba),
                        });
                        self.present();
                    }
                    self.flight = None;
                }
            }
            return;
        }
        self.flight = Some(host.session().debug_live_images_begin(true));
    }

    fn present(&mut self) {
        let Some(picture) = self.picture.as_ref() else {
            return;
        };
        let (w, h) = (picture.width, picture.height);
        let scale = if w < 200 { 2u32 } else { 1 };
        let rep: Option<Retained<NSBitmapImageRep>> = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
                    self.mtm.alloc::<NSBitmapImageRep>(),
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
        };
        if let Some(rep) = rep {
            unsafe {
                let dst = rep.bitmapData();
                std::ptr::copy_nonoverlapping(picture.bgrx.as_ptr(), dst, picture.bgrx.len());
            }
            let image = NSImage::initWithSize(
                NSImage::alloc(),
                NSSize::new(f64::from(w * scale), f64::from(h * scale)),
            );
            image.addRepresentation(&rep);
            self.image_view.setImage(Some(&image));
        }
    }
}
