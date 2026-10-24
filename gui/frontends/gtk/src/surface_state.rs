//! Native-surface liveness predicates shared by every widget owner.
//!
//! Leaf module by design: only GTK/GDK types, never sibling frontend
//! modules, so debugger/OSD/window can all use these without creating
//! module cycles.

use gtk::prelude::*;

/// True when the window has no usable native surface: never realized,
/// or destroyed out from under the widget by the window manager. Widget
/// signals never report this (external destruction skips close-request
/// and `destroy` never fires), so every reopen path must check
/// explicitly instead of assuming a live surface.
pub(crate) fn surface_gone(window: &impl IsA<gtk::Window>) -> bool {
    use gtk::gdk::prelude::SurfaceExt as _;
    window
        .upcast_ref::<gtk::Window>()
        .surface()
        .is_none_or(|surface| surface.is_destroyed())
}

/// True when touching the window issues X requests on a dead surface: a
/// surface exists and GDK reports it destroyed. A window with no surface
/// yet (never realized) or an already-disposed widget has nothing to
/// query, so touching it is safe; reopen paths still treat those as
/// unusable via [`surface_gone`]. Periodic refreshes use this: they must
/// run for never-realized dialogs and stop only for dead surfaces.
pub(crate) fn surface_dead(window: &impl IsA<gtk::Window>) -> bool {
    use gtk::gdk::prelude::SurfaceExt as _;
    window
        .upcast_ref::<gtk::Window>()
        .surface()
        .is_some_and(|surface| surface.is_destroyed())
}
