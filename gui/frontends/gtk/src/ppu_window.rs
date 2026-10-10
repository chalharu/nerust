//! GTK PPU viewer: system images (pattern tables) at 2x with tile
//! hover readout. Follows the debugger refresh; Sync derives scaled
//! bytes once per refresh, never per hover.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use nerust_gui_viewmodel::debugger::{PPU_HOVER_HINT, scale2x_nearest};

use super::State;

pub(crate) type PpuWindow = Rc<RefCell<PpuWindowCore>>;

pub(crate) struct PpuWindowCore {
    images_box: gtk::Box,
    hover_label: gtk::Label,
}

impl PpuWindowCore {
    pub(crate) fn open(
        app: &gtk::Application,
        main_window: &gtk::ApplicationWindow,
        _state: Rc<RefCell<State>>,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        panels: String,
    ) -> PpuWindow {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("PPU Viewer")
            .default_width(560)
            .default_height(620)
            .build();
        window.set_transient_for(Some(main_window));
        let images_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
        images_box.set_margin_top(16);
        images_box.set_margin_bottom(16);
        images_box.set_margin_start(16);
        images_box.set_margin_end(16);
        let hover_label = gtk::Label::new(Some(PPU_HOVER_HINT));
        hover_label.set_xalign(0.0);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_overlay_scrolling(false);
        scroll.set_child(Some(&images_box));
        scroll.set_vexpand(true);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.append(&hover_label);
        root.append(&scroll);
        window.set_child(Some(&root));
        window.connect_close_request(|window| {
            window.set_visible(false);
            glib::Propagation::Stop
        });
        let result = Rc::new(RefCell::new(Self {
            images_box,
            hover_label,
        }));
        result.borrow_mut().sync(&images, &panels);
        window.present();
        result
    }

    /// Re-derive scaled pictures from fresh images (one 2x scale per
    /// refresh). Empty images show an explicit degenerate, never a
    /// blank window.
    pub(crate) fn sync(&mut self, images: &[(String, u32, u32, Vec<u8>)], panels: &str) {
        while let Some(child) = self.images_box.first_child() {
            self.images_box.remove(&child);
        }
        if images.is_empty() {
            let empty = gtk::Label::new(Some("(no images)"));
            empty.set_xalign(0.0);
            self.images_box.append(&empty);
        }
        for (label, width, height, rgba) in images {
            let title = gtk::Label::new(Some(label));
            title.set_xalign(0.0);
            self.images_box.append(&title);
            let scaled = scale2x_nearest(rgba, *width, *height);
            let stride = (*width as usize) * 2 * 4;
            let texture = gdk::MemoryTexture::new(
                *width as i32 * 2,
                *height as i32 * 2,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from(&scaled),
                stride,
            );
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_can_shrink(false);
            picture.set_size_request(*width as i32 * 2, *height as i32 * 2);
            let motion = gtk::EventControllerMotion::new();
            let hover = self.hover_label.clone();
            let label = label.clone();
            let (width, height) = (*width, *height);
            motion.connect_motion(move |_, x, y| {
                hover.set_label(&tile_hover_text(&label, width, height, x, y));
            });
            picture.add_controller(motion);
            self.images_box.append(&picture);
        }
        // Panels live in the PPU viewer on every frontend, matching
        // the tao viewer layout (images first, panels below).
        let panels_title = gtk::Label::new(Some("Panels"));
        panels_title.set_xalign(0.0);
        self.images_box.append(&panels_title);
        let panels_label = gtk::Label::new(Some(panels));
        panels_label.add_css_class("monospace");
        panels_label.set_xalign(0.0);
        self.images_box.append(&panels_label);
    }
}

/// Tile hover line: 2x display scale, 8x8 tiles, byte offset of the
/// tile start (+16 per tile). Same geometry as the tao viewer.
fn tile_hover_text(label: &str, width: u32, height: u32, x: f64, y: f64) -> String {
    let sx = (x / 2.0).floor().clamp(0.0, width as f64 - 1.0) as u32;
    let sy = (y / 2.0).floor().clamp(0.0, height as f64 - 1.0) as u32;
    let cols = (width / 8).max(1);
    let idx = (sy / 8) * cols + (sx / 8);
    format!("{label} tile #{idx} (px {sx},{sy}, +{:04X})", idx * 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_hover_matches_tao_geometry() {
        assert_eq!(
            tile_hover_text("Pattern left", 128, 128, 88.0, 26.0),
            "Pattern left tile #21 (px 44,13, +0150)"
        );
    }
}
