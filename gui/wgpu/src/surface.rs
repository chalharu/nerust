// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nerust_screen_traits::PhysicalSize;
use nerust_wgpuwrap::SurfaceSize;
use std::sync::Arc;
use tao::{dpi::PhysicalSize as TaoPhysicalSize, window::Window as TaoWindow};
use wgpu::{Instance, Surface};
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
        HandleError, RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
        XlibDisplayHandle, XlibWindowHandle,
    },
    std::ptr::NonNull,
    tao::platform::unix::WindowExtUnix,
};

pub(crate) struct RenderSurface {
    // The surface must drop before the target widget/handles and their backing instance.
    surface: Surface<'static>,
    surface_target: SurfaceTarget,
    instance: Instance,
}

impl RenderSurface {
    pub(crate) fn new(surface_target: SurfaceTarget) -> Result<Self, String> {
        let instance = Instance::default();
        surface_target.prepare();
        let surface = surface_target.create_surface(&instance)?;
        Ok(Self {
            surface,
            surface_target,
            instance,
        })
    }

    pub(crate) fn surface(&self) -> &Surface<'static> {
        &self.surface
    }

    pub(crate) fn instance(&self) -> &Instance {
        &self.instance
    }

    pub(crate) fn surface_size(&self, fallback: TaoPhysicalSize<u32>) -> SurfaceSize {
        self.surface_target.surface_size(fallback)
    }

    pub(crate) fn recreate_surface(&mut self) -> Result<(), String> {
        self.surface_target.prepare();
        self.surface = self.surface_target.create_surface(&self.instance)?;
        Ok(())
    }
}

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
    pub(crate) fn new(window: Arc<TaoWindow>, content_size: PhysicalSize) -> Self {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            Self {
                kind: SurfaceTargetKind::Gtk(GtkRenderTarget::new(&window, content_size)),
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

    fn surface_size(&self, fallback: TaoPhysicalSize<u32>) -> SurfaceSize {
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

    fn create_surface(&self, instance: &Instance) -> Result<Surface<'static>, String> {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            match &self.kind {
                SurfaceTargetKind::Gtk(target) => unsafe {
                    let raw_display_handle = target
                        .raw_display_handle()
                        .map_err(|err| format!("failed to acquire raw display handle: {err:?}"))?;
                    let raw_window_handle = target
                        .raw_window_handle()
                        .map_err(|err| format!("failed to acquire raw window handle: {err:?}"))?;
                    instance
                        .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                            raw_display_handle: Some(raw_display_handle),
                            raw_window_handle,
                        })
                        .map_err(|err| format!("failed to create wgpu surface: {err:?}"))
                },
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
                SurfaceTargetKind::Window(window) => instance
                    .create_surface(window.clone())
                    .map_err(|err| format!("failed to create wgpu surface: {err:?}")),
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
    fn new(window: &TaoWindow, content_size: PhysicalSize) -> Self {
        let widget = EventBox::new();
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_size_request(content_size.width as i32, content_size.height as i32);
        window
            .default_vbox()
            .expect("tao default_vbox must exist for Linux menu integration")
            .pack_start(&widget, true, true, 0);

        Self { widget }
    }

    fn prepare(&self) {
        self.widget.realize();
    }

    fn surface_size(&self, fallback: TaoPhysicalSize<u32>) -> SurfaceSize {
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
