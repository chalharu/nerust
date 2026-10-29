//! Owned, native-free window presentation, separate from native lifetime identity.
//!
//! Clone a window under the owner's borrow, release it, then visit the snapshot:
//! native callbacks may synchronously reenter the host or auxiliary registry.

use crate::{chrome::aux::AuxId, chrome::menu::Menubar};

/// Semantic destination. Generation-sensitive callbacks must still use `AuxId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowId {
    Main,
    Auxiliary(AuxId),
}

#[derive(Debug, Clone)]
pub struct Window {
    pub id: WindowId,
    pub title: String,
    pub menu: Option<Menubar>,
    pub contents: Widget,
}

/// Concrete native work after a Host operation. No domain result leaks.
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
    Controls(Box<crate::scene::control::ControlScreen>),
    /// Reservation exists, but no generic control screen has been supplied yet.
    PendingControls,
}

/// Grouped main surface. Renderer resources remain native-owned.
#[derive(Debug, Clone, Default)]
pub struct MainContents;

impl Window {
    /// Fully generic auxiliary window: the view definition owns the kind and
    /// title, the framework only tracks the generation-sensitive `AuxId`.
    pub fn controls(id: AuxId, screen: crate::scene::control::ControlScreen) -> Self {
        Self {
            id: WindowId::Auxiliary(id),
            title: screen.title.clone(),
            menu: None,
            contents: Widget {
                contents: WidgetContents::Controls(Box::new(screen)),
            },
        }
    }

    pub fn main(title: &str) -> Self {
        Self {
            id: WindowId::Main,
            title: title.into(),
            menu: None,
            contents: Widget {
                contents: WidgetContents::Main(MainContents),
            },
        }
    }

    /// Native creation may read this immediately after `request_open`.
    /// Control windows replace the pending presentation with their own window
    /// before rendering; no default editor or second draft is created.
    pub fn auxiliary(id: AuxId, title: &str) -> Self {
        Self {
            id: WindowId::Auxiliary(id),
            title: title.into(),
            menu: None,
            contents: Widget {
                contents: WidgetContents::PendingControls,
            },
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
    fn visit_controls(&mut self, _screen: &crate::scene::control::ControlScreen) {}
    fn visit_pending_controls(&mut self) {}
}

/// Window chrome sync decision. Frontends apply `set_fullscreen` to the
/// native window and `restore_size` via their default-size API; the scaling
/// policy behind the restore stays view-side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowChromeSync {
    pub restore_size: Option<(i32, i32)>,
    pub set_fullscreen: Option<bool>,
}
