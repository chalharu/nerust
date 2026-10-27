//! Owned, native-free window presentation, separate from native lifetime identity.
//!
//! Clone a window under the owner's borrow, release it, then visit the snapshot:
//! native callbacks may synchronously reenter the host or auxiliary registry.

use crate::{focus::AuxKind, menu::Menubar, settings::SettingsScreen};

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

#[derive(Debug, Clone)]
pub struct Widget {
    pub contents: WidgetContents,
}

#[derive(Debug, Clone)]
pub enum WidgetContents {
    Main(MainContents),
    Settings(Box<SettingsScreen>),
    /// Reservation exists, but no controller screen has been supplied yet.
    PendingSettings,
    Debug(DebugContents),
    Ppu(PpuContents),
    History(HistoryContents),
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

/// Grouped history presentation; HistoryModel/HistoryVisitor remain reusable.
#[derive(Debug, Clone, Default)]
pub struct HistoryContents;

impl Window {
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
    /// Settings must replace the pending presentation with `controller.window()`
    /// before rendering; no default editor or second settings draft is created.
    pub fn auxiliary(kind: AuxKind) -> Self {
        let (title, contents) = match kind {
            AuxKind::Debug => ("Debugger", WidgetContents::Debug(DebugContents)),
            AuxKind::Ppu => ("PPU Viewer", WidgetContents::Ppu(PpuContents)),
            AuxKind::History => (
                "Notification History",
                WidgetContents::History(HistoryContents),
            ),
            AuxKind::Settings => ("Settings", WidgetContents::PendingSettings),
        };
        Self {
            id: WindowId::Auxiliary(kind),
            title: title.into(),
            menu: None,
            contents: Widget { contents },
        }
    }

    pub fn settings(screen: SettingsScreen) -> Self {
        Self {
            id: WindowId::Auxiliary(AuxKind::Settings),
            title: screen.title.clone(),
            menu: None,
            contents: Widget {
                contents: WidgetContents::Settings(Box::new(screen)),
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
            WidgetContents::Settings(screen) => visitor.visit_settings(screen),
            WidgetContents::PendingSettings => visitor.visit_pending_settings(),
            WidgetContents::Debug(contents) => visitor.visit_debug(contents),
            WidgetContents::Ppu(contents) => visitor.visit_ppu(contents),
            WidgetContents::History(contents) => visitor.visit_history(contents),
        }
        visitor.end_widget(self);
    }
}

/// Object-safe outer traversal, with no Send/Sync or toolkit constraints.
/// `visit_settings` can call `screen.visit(&mut dyn SettingsVisitor)`; similarly,
/// `visit_menu` can delegate to the existing menu presenter/visitor.
pub trait WindowVisitor {
    fn begin_window(&mut self, _window: &Window) {}
    fn end_window(&mut self, _window: &Window) {}
    fn visit_menu(&mut self, _menu: &Menubar) {}
    fn begin_widget(&mut self, _widget: &Widget) {}
    fn end_widget(&mut self, _widget: &Widget) {}
    fn visit_main(&mut self, _contents: &MainContents) {}
    fn visit_settings(&mut self, _screen: &SettingsScreen) {}
    fn visit_pending_settings(&mut self) {}
    fn visit_debug(&mut self, _contents: &DebugContents) {}
    fn visit_ppu(&mut self, _contents: &PpuContents) {}
    fn visit_history(&mut self, _contents: &HistoryContents) {}
}

#[cfg(test)]
mod tests;
