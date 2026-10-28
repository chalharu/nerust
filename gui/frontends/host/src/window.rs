//! Owned, native-free window presentation, separate from native lifetime identity.
//!
//! Clone a window under the owner's borrow, release it, then visit the snapshot:
//! native callbacks may synchronously reenter the host or auxiliary registry.

use crate::{focus::AuxKind, menu::Menubar};

/// Semantic destination. Generation-sensitive callbacks must still use `AuxId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowId {
    Main,
    Auxiliary(AuxKind),
}

#[derive(Debug, Clone)]
pub struct Window {
    pub id: WindowId,
    pub title: String,
    pub menu: Option<Menubar>,
    pub contents: Widget,
}

/// Concrete native work after a Host operation. No settings domain result leaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowEffects {
    pub rebuild_renderer: bool,
    pub fullscreen: Option<bool>,
    /// Logical client dimensions, without toolkit decorations or menu height.
    pub resize: Option<(u32, u32)>,
    pub redraw: bool,
    pub refresh_chrome: bool,
}

#[derive(Debug, Clone)]
pub struct Widget {
    pub contents: WidgetContents,
}

#[derive(Debug, Clone)]
pub enum WidgetContents {
    Main(MainContents),
    Controls(Box<wakui::control::ControlScreen>),
    /// Reservation exists, but no generic control screen has been supplied yet.
    PendingControls,
    Debug(DebugContents),
    Ppu(PpuContents),
}

/// Grouped emulation surface. Renderer resources remain native-owned.
#[derive(Debug, Clone, Default)]
pub struct MainContents;

/// Grouped debugger presentation, deliberately not a per-control widget tree.
/// DebuggerDriver/models still own semantic debugger state; native controls and
/// their opaque toolkit state remain in the adapter's existing auxiliary handle.
#[derive(Debug, Clone, Default)]
pub struct DebugContents;

/// Grouped PPU presentation. Probe/model state stays in the existing debugger
/// pipeline; textures and toolkit state stay in the native auxiliary handle.
#[derive(Debug, Clone, Default)]
pub struct PpuContents;

impl Window {
    pub fn controls(kind: AuxKind, screen: wakui::control::ControlScreen) -> Self {
        Self {
            id: WindowId::Auxiliary(kind),
            title: screen.title.clone(),
            menu: None,
            contents: Widget {
                contents: WidgetContents::Controls(Box::new(screen)),
            },
        }
    }

    pub fn main() -> Self {
        Self {
            id: WindowId::Main,
            title: "nerust".into(),
            menu: None,
            contents: Widget {
                contents: WidgetContents::Main(MainContents),
            },
        }
    }

    /// Native creation may read this immediately after `request_open`.
    /// Control windows replace the pending presentation with `session.window()`
    /// before rendering; no default editor or second settings draft is created.
    pub fn auxiliary(kind: AuxKind) -> Self {
        let (title, contents) = match kind {
            AuxKind::Debug => ("Debugger", WidgetContents::Debug(DebugContents)),
            AuxKind::Ppu => ("PPU Viewer", WidgetContents::Ppu(PpuContents)),
            AuxKind::History => ("Notification History", WidgetContents::PendingControls),
            AuxKind::Settings => ("Settings", WidgetContents::PendingControls),
        };
        Self {
            id: WindowId::Auxiliary(kind),
            title: title.into(),
            menu: None,
            contents: Widget { contents },
        }
    }

    pub fn visit(&self, visitor: &mut dyn WindowVisitor) {
        visitor.begin_window(self);
        if let Some(menu) = &self.menu {
            visitor.visit_menu(menu);
        }
        self.contents.visit(visitor);
        visitor.end_window(self);
    }
}

impl Widget {
    pub fn visit(&self, visitor: &mut dyn WindowVisitor) {
        visitor.begin_widget(self);
        match &self.contents {
            WidgetContents::Main(contents) => visitor.visit_main(contents),
            WidgetContents::Controls(screen) => visitor.visit_controls(screen),
            WidgetContents::PendingControls => visitor.visit_pending_controls(),
            WidgetContents::Debug(contents) => visitor.visit_debug(contents),
            WidgetContents::Ppu(contents) => visitor.visit_ppu(contents),
        }
        visitor.end_widget(self);
    }
}

/// Object-safe outer traversal, with no Send/Sync or toolkit constraints.
/// `visit_controls` can call `screen.visit(&mut dyn ControlVisitor)`; similarly,
/// `visit_menu` can delegate to the existing menu presenter/visitor.
pub trait WindowVisitor {
    fn begin_window(&mut self, _window: &Window) {}
    fn end_window(&mut self, _window: &Window) {}
    fn visit_menu(&mut self, _menu: &Menubar) {}
    fn begin_widget(&mut self, _widget: &Widget) {}
    fn end_widget(&mut self, _widget: &Widget) {}
    fn visit_main(&mut self, _contents: &MainContents) {}
    fn visit_controls(&mut self, _screen: &wakui::control::ControlScreen) {}
    fn visit_pending_controls(&mut self) {}
    fn visit_debug(&mut self, _contents: &DebugContents) {}
    fn visit_ppu(&mut self, _contents: &PpuContents) {}
}

/// Window chrome sync decision. Frontends apply `set_fullscreen` to the
/// native window and `restore_size` via their default-size API; the scaling
/// policy behind the restore stays view-side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowChromeSync {
    pub restore_size: Option<(i32, i32)>,
    pub set_fullscreen: Option<bool>,
}
