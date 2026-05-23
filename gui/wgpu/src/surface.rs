// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nerust_backend_wgpu::{RenderSurfaceTarget, SurfaceSize};
use nerust_gui_shell::shell_api::WindowSize;
use raw_window_handle::{HandleError, RawDisplayHandle, RawWindowHandle};
#[cfg(not(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
)))]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::sync::Arc;
use tao::window::Window as TaoWindow;
#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
use {
    gtk::{
        EventBox,
        gdk::prelude::DisplayExtManual,
        prelude::{BoxExt, ObjectType, WidgetExt},
    },
    raw_window_handle::{
        WaylandDisplayHandle, WaylandWindowHandle, XlibDisplayHandle, XlibWindowHandle,
    },
    std::ptr::NonNull,
    tao::platform::unix::WindowExtUnix,
};

pub(crate) struct SurfaceTarget {
    kind: SurfaceTargetKind,
}

enum SurfaceTargetKind {
    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    Window(Arc<TaoWindow>),
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    Gtk(GtkRenderTarget),
}

impl SurfaceTarget {
    pub(crate) fn new(window: Arc<TaoWindow>, content_size: WindowSize) -> Self {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            Self {
                kind: SurfaceTargetKind::Gtk(GtkRenderTarget::new(window, content_size)),
            }
        }

        #[cfg(not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        )))]
        {
            let _ = content_size;
            Self {
                kind: SurfaceTargetKind::Window(window),
            }
        }
    }
}

// Safety: `SurfaceTarget` owns the platform objects backing the raw handles it
// returns, and those objects remain alive for the lifetime of the corresponding
// render surface built by the backend.
unsafe impl RenderSurfaceTarget for SurfaceTarget {
    fn prepare(&self) {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        match &self.kind {
            SurfaceTargetKind::Gtk(target) => target.prepare(),
        }
    }

    fn surface_size(&self, fallback: SurfaceSize) -> SurfaceSize {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            match &self.kind {
                SurfaceTargetKind::Gtk(target) => target.surface_size(fallback),
            }
        }

        #[cfg(not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        )))]
        {
            SurfaceSize::new(fallback.width, fallback.height)
        }
    }

    fn raw_window_handle(&self) -> Result<RawWindowHandle, HandleError> {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            match &self.kind {
                SurfaceTargetKind::Gtk(target) => target.raw_window_handle(),
            }
        }

        #[cfg(not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        )))]
        {
            match &self.kind {
                SurfaceTargetKind::Window(window) => {
                    window.window_handle().map(|handle| handle.as_raw())
                }
            }
        }
    }

    fn raw_display_handle(&self) -> Result<Option<RawDisplayHandle>, HandleError> {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            match &self.kind {
                SurfaceTargetKind::Gtk(target) => target.raw_display_handle().map(Some),
            }
        }

        #[cfg(not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        )))]
        {
            match &self.kind {
                SurfaceTargetKind::Window(window) => {
                    window.display_handle().map(|handle| Some(handle.as_raw()))
                }
            }
        }
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
struct GtkRenderTarget {
    _window: Arc<TaoWindow>,
    widget: EventBox,
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
impl GtkRenderTarget {
    fn new(window: Arc<TaoWindow>, content_size: WindowSize) -> Self {
        let widget = EventBox::new();
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_size_request(content_size.width as i32, content_size.height as i32);
        window
            .default_vbox()
            .expect("tao default_vbox must exist for Linux menu integration")
            .pack_start(&widget, true, true, 0);

        Self {
            _window: window,
            widget,
        }
    }

    fn prepare(&self) {
        self.widget.realize();
    }

    fn surface_size(&self, fallback: SurfaceSize) -> SurfaceSize {
        let width = self.widget.allocated_width();
        let height = self.widget.allocated_height();
        if width > 0 && height > 0 {
            let scale = self.widget.scale_factor().max(1) as u32;
            SurfaceSize::new(width as u32 * scale, height as u32 * scale)
        } else {
            SurfaceSize::new(fallback.width, fallback.height)
        }
    }

    fn gdk_window(&self) -> Result<gtk::gdk::Window, HandleError> {
        self.widget.window().ok_or(HandleError::Unavailable)
    }

    fn is_wayland(&self) -> bool {
        self.widget.display().backend().is_wayland()
    }

    fn raw_window_handle(&self) -> Result<RawWindowHandle, HandleError> {
        let window = self.gdk_window()?;
        if self.is_wayland() {
            let surface = unsafe {
                gdk_wayland_sys::gdk_wayland_window_get_wl_surface(window.as_ptr() as *mut _)
            };
            let surface = NonNull::new(surface)
                .ok_or(HandleError::Unavailable)?
                .cast();
            Ok(RawWindowHandle::Wayland(WaylandWindowHandle::new(surface)))
        } else {
            let xid = unsafe { gdk_x11_sys::gdk_x11_window_get_xid(window.as_ptr() as *mut _) };
            Ok(RawWindowHandle::Xlib(XlibWindowHandle::new(xid)))
        }
    }

    fn raw_display_handle(&self) -> Result<RawDisplayHandle, HandleError> {
        let display = self.widget.display();
        if self.is_wayland() {
            let display = unsafe {
                gdk_wayland_sys::gdk_wayland_display_get_wl_display(display.as_ptr() as *mut _)
            };
            let display = NonNull::new(display)
                .ok_or(HandleError::Unavailable)?
                .cast();
            Ok(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                display,
            )))
        } else {
            let display =
                unsafe { gdk_x11_sys::gdk_x11_display_get_xdisplay(display.as_ptr() as *mut _) };
            let display = NonNull::new(display as *mut _).ok_or(HandleError::Unavailable)?;
            let screen = self.widget.screen().ok_or(HandleError::Unavailable)?;
            let screen =
                unsafe { gdk_x11_sys::gdk_x11_screen_get_screen_number(screen.as_ptr() as *mut _) }
                    as _;
            Ok(RawDisplayHandle::Xlib(XlibDisplayHandle::new(
                Some(display),
                screen,
            )))
        }
    }
}
