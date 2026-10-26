//! XAML-backed [`WinSurface`](nerust_win::WinSurface): native effects
//! become pending requests the UI thread drains on its tick, so the
//! shared host never touches XAML objects directly.

use std::path::PathBuf;

/// Pending native effects for the UI thread to execute.
#[derive(Debug, Default)]
pub struct WinUiSurface {
    pub titles: Vec<String>,
    pub fullscreen_calls: Vec<bool>,
    pub modal_errors: Vec<String>,
    pub debugger_requests: u32,
    /// File chosen through the WinUI picker is consumed by the UI layer
    /// directly (async); this seam only records the request path.
    pub rom_pick: Option<PathBuf>,
}

impl nerust_win::WinSurface for WinUiSurface {
    fn set_title(&mut self, title: &str) {
        self.titles.push(title.to_string());
    }

    fn set_fullscreen(&mut self, fullscreen: bool) {
        self.fullscreen_calls.push(fullscreen);
    }

    fn pick_rom_file(&mut self, _filters: &[(String, Vec<String>)]) -> Option<PathBuf> {
        self.rom_pick.clone()
    }

    fn show_error_modal(&mut self, detail: &str) {
        self.modal_errors.push(detail.to_string());
    }

    fn request_debugger(&mut self) {
        self.debugger_requests += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_win::WinSurface;

    #[test]
    fn surface_queues_effects_for_ui_thread() {
        let mut surface = WinUiSurface::default();
        surface.set_title("t");
        surface.set_fullscreen(true);
        surface.show_error_modal("e");
        surface.request_debugger();
        assert_eq!(surface.titles, ["t"]);
        assert_eq!(surface.fullscreen_calls, [true]);
        assert_eq!(surface.modal_errors, ["e"]);
        assert_eq!(surface.debugger_requests, 1);
        assert_eq!(surface.pick_rom_file(&[]), None);
    }
}
