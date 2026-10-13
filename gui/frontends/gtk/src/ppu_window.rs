//! GTK PPU viewer: system images (pattern tables) at 2x with tile
//! hover readout. Follows the debugger refresh; Sync derives scaled
//! bytes once per refresh, never per hover.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use nerust_core_traits::debugger::DebugPanel;
use nerust_gui_viewmodel::debugger::{format_panels, scale2x_nearest};
use nerust_settings_core::i18n::{UiText, text, tile_hover};
use nerust_settings_core::model::language::AppLanguage;

use super::State;

pub(crate) type PpuWindow = Rc<RefCell<PpuWindowCore>>;

pub(crate) struct PpuWindowCore {
    window: gtk::ApplicationWindow,
    images_box: gtk::Box,
    hover_label: gtk::Label,
    language: AppLanguage,
    /// Generation of the last synced images. Refresh drains re-run
    /// sync with identical generations while paused; the 2x scale
    /// and widget rebuild run only on change. Starts impossibly high
    /// so the first sync always derives (generation zero is valid).
    last_gen: u64,
    last_panels: Vec<DebugPanel>,
}

impl PpuWindowCore {
    pub(crate) fn open(
        app: &gtk::Application,
        main_window: &gtk::ApplicationWindow,
        _state: Rc<RefCell<State>>,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        images_gen: u64,
        panels: Vec<DebugPanel>,
        language: AppLanguage,
    ) -> PpuWindow {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(text(language, UiText::PpuViewer))
            .default_width(560)
            .default_height(620)
            .build();
        window.set_transient_for(Some(main_window));
        let images_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
        images_box.set_margin_top(16);
        images_box.set_margin_bottom(16);
        images_box.set_margin_start(16);
        images_box.set_margin_end(16);
        let hover_label = gtk::Label::new(Some(text(language, UiText::HoverTile)));
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
            window: window.clone(),
            images_box,
            hover_label,
            language,
            last_gen: u64::MAX,
            last_panels: Vec::new(),
        }));
        result.borrow_mut().sync(&images, images_gen, &panels);
        window.present();
        result
    }

    /// Re-present a hidden viewer (idempotent reopen path).
    pub(crate) fn present(&self) {
        self.window.present();
    }

    /// Re-derive scaled pictures from fresh images (one 2x scale per
    /// change). Empty images show an explicit degenerate, never a
    /// blank window. Unchanged content skips the rebuild entirely.
    pub(crate) fn sync(
        &mut self,
        images: &[(String, u32, u32, Vec<u8>)],
        images_gen: u64,
        panels: &[DebugPanel],
    ) {
        if images_gen == self.last_gen && panels == self.last_panels {
            return;
        }
        self.last_gen = images_gen;
        self.last_panels = panels.to_vec();
        while let Some(child) = self.images_box.first_child() {
            self.images_box.remove(&child);
        }
        if images.is_empty() {
            let empty = gtk::Label::new(Some(text(self.language, UiText::NoImages)));
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
            let language = self.language;
            motion.connect_motion(move |_, x, y| {
                hover.set_label(&tile_hover_text(language, &label, width, height, x, y));
            });
            picture.add_controller(motion);
            self.images_box.append(&picture);
        }
        // Panels live in the PPU viewer on every frontend, matching
        // the tao viewer layout (images first, panels below).
        // Systems without observation tables show no Panels section
        // at all instead of a puzzling marker.
        if !panels.is_empty() {
            let panels_title = gtk::Label::new(Some(text(self.language, UiText::Panels)));
            panels_title.set_xalign(0.0);
            self.images_box.append(&panels_title);
            let panels_label = gtk::Label::new(Some(&format_panels(panels)));
            panels_label.add_css_class("monospace");
            panels_label.set_xalign(0.0);
            self.images_box.append(&panels_label);
        }
    }
}

/// Tile hover line: 2x display scale, 8x8 tiles, byte offset of the
/// tile start (+16 per tile). Same geometry as the tao viewer.
fn tile_hover_text(
    language: AppLanguage,
    label: &str,
    width: u32,
    height: u32,
    x: f64,
    y: f64,
) -> String {
    let sx = (x / 2.0).floor().clamp(0.0, width as f64 - 1.0) as u32;
    let sy = (y / 2.0).floor().clamp(0.0, height as f64 - 1.0) as u32;
    let cols = (width / 8).max(1);
    let idx = (sy / 8) * cols + (sx / 8);
    tile_hover(language, label, idx, sx, sy, idx * 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_hover_matches_tao_geometry() {
        assert_eq!(
            tile_hover_text(AppLanguage::English, "Pattern left", 128, 128, 88.0, 26.0),
            "Pattern left tile #21 (px 44,13, +0x0150)"
        );
    }
}
