use crate::{GtkHost, Parent, View, gtk};
#[cfg(target_os = "linux")]
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use panechord_runtime::{
    Binding, ImageFrame, KeyCode, Session, WindowEvent, WindowHooks, WindowHost, WindowSurface,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use std::{cell::RefCell, collections::HashSet, rc::Rc};

pub(crate) type ViewportHook = Rc<dyn Fn(f32, f32)>;

struct Surface(gtk::Window);

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn gdk_x11_display_get_xdisplay(
        display: *mut gtk::gdk::ffi::GdkDisplay,
    ) -> *mut std::ffi::c_void;
    fn gdk_x11_surface_get_xid(surface: *mut gtk::gdk::ffi::GdkSurface) -> std::ffi::c_ulong;
    fn gdk_wayland_display_get_wl_display(
        display: *mut gtk::gdk::ffi::GdkDisplay,
    ) -> *mut std::ffi::c_void;
    fn gdk_wayland_surface_get_wl_surface(
        surface: *mut gtk::gdk::ffi::GdkSurface,
    ) -> *mut std::ffi::c_void;
}

impl HasDisplayHandle for Surface {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        #[cfg(target_os = "linux")]
        {
            use raw_window_handle::{RawDisplayHandle, WaylandDisplayHandle, XlibDisplayHandle};
            let display = gtk::prelude::WidgetExt::display(&self.0);
            let raw = if display.type_().name().contains("X11") {
                let pointer = unsafe { gdk_x11_display_get_xdisplay(display.to_glib_none().0) };
                RawDisplayHandle::Xlib(XlibDisplayHandle::new(
                    Some(std::ptr::NonNull::new(pointer).ok_or(HandleError::Unavailable)?),
                    0,
                ))
            } else if display.type_().name().contains("Wayland") {
                let pointer =
                    unsafe { gdk_wayland_display_get_wl_display(display.to_glib_none().0) };
                RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                    std::ptr::NonNull::new(pointer).ok_or(HandleError::Unavailable)?,
                ))
            } else {
                return Err(HandleError::NotSupported);
            };
            Ok(unsafe { DisplayHandle::borrow_raw(raw) })
        }
        #[cfg(not(target_os = "linux"))]
        Err(HandleError::NotSupported)
    }
}

impl HasWindowHandle for Surface {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        #[cfg(target_os = "linux")]
        {
            use raw_window_handle::{RawWindowHandle, WaylandWindowHandle, XlibWindowHandle};
            let surface = self.0.surface().ok_or(HandleError::Unavailable)?;
            let raw = if surface.type_().name().contains("X11") {
                let xid = unsafe { gdk_x11_surface_get_xid(surface.to_glib_none().0) };
                if xid == 0 {
                    return Err(HandleError::Unavailable);
                }
                RawWindowHandle::Xlib(XlibWindowHandle::new(xid))
            } else if surface.type_().name().contains("Wayland") {
                let pointer =
                    unsafe { gdk_wayland_surface_get_wl_surface(surface.to_glib_none().0) };
                RawWindowHandle::Wayland(WaylandWindowHandle::new(
                    std::ptr::NonNull::new(pointer).ok_or(HandleError::Unavailable)?,
                ))
            } else {
                return Err(HandleError::NotSupported);
            };
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
        #[cfg(not(target_os = "linux"))]
        Err(HandleError::NotSupported)
    }
}

impl WindowSurface for Surface {
    fn scale_factor(&self) -> f64 {
        f64::from(self.0.scale_factor().max(1))
    }
    fn toggle_fullscreen(&self) {
        self.0.set_fullscreened(!self.0.is_fullscreen());
    }
}

impl WindowHost<View> for GtkHost {
    fn register_view(
        &self,
        session: &Session,
        id: &str,
        view: &View,
        hooks: WindowHooks,
    ) -> Result<(), String> {
        panechord_runtime::UiHost::attach(self, session.clone())?;
        self.register_view_with_hooks(session, id, view, hooks)
    }
}

fn key_code(key: gtk::gdk::Key) -> KeyCode {
    if let Some(character) = key.to_unicode() {
        if character.is_ascii_alphabetic() {
            return KeyCode::new(format!("Key{}", character.to_ascii_uppercase()));
        }
        if character.is_ascii_digit() {
            return KeyCode::new(format!("Digit{character}"));
        }
    }
    let name = key.name().map(|name| name.to_string()).unwrap_or_default();
    let canonical = match name.as_str() {
        "Return" => "Enter",
        "Escape" => "Escape",
        "space" => "Space",
        "Tab" | "ISO_Left_Tab" => "Tab",
        "BackSpace" => "Backspace",
        "Up" => "ArrowUp",
        "Down" => "ArrowDown",
        "Left" => "ArrowLeft",
        "Right" => "ArrowRight",
        "Shift_L" => "ShiftLeft",
        "Shift_R" => "ShiftRight",
        "Control_L" => "ControlLeft",
        "Control_R" => "ControlRight",
        "Alt_L" => "AltLeft",
        "Alt_R" => "AltRight",
        "Meta_L" | "Super_L" => "MetaLeft",
        "Meta_R" | "Super_R" => "MetaRight",
        "comma" | "less" => "Comma",
        "period" | "greater" => "Period",
        "slash" | "question" => "Slash",
        "semicolon" | "colon" => "Semicolon",
        "apostrophe" | "quotedbl" => "Quote",
        "bracketleft" | "braceleft" => "BracketLeft",
        "bracketright" | "braceright" => "BracketRight",
        "backslash" | "bar" => "Backslash",
        "minus" | "underscore" => "Minus",
        "equal" => "Equal",
        "plus" => "Plus",
        "grave" | "asciitilde" => "Backquote",
        "Page_Up" => "PageUp",
        "Page_Down" => "PageDown",
        "Caps_Lock" => "CapsLock",
        "Scroll_Lock" => "ScrollLock",
        "Print" => "PrintScreen",
        "KP_Enter" => "NumpadEnter",
        "KP_Add" => "NumpadAdd",
        "KP_Subtract" => "NumpadSubtract",
        "KP_Multiply" => "NumpadMultiply",
        "KP_Divide" => "NumpadDivide",
        "KP_Decimal" => "NumpadDecimal",
        _ => name.as_str(),
    };
    KeyCode::new(canonical)
}

impl View {
    pub fn on_surface_ready(
        &self,
        callback: impl FnOnce(Rc<dyn WindowSurface>) -> Result<(), String> + 'static,
    ) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        gtk::prelude::WidgetExt::realize(&self.0.window);
        callback(Rc::new(Surface(self.0.window.clone())))
    }

    pub fn on_surface_viewport(&self, callback: ViewportHook) {
        self.0.surface_viewport.replace(Some(callback.clone()));
        let (width, height) = self.0.surface_size.get();
        if width > 0.0 && height > 0.0 {
            callback(width, height);
        }
    }

    pub fn on_surface_event(&self, callback: Rc<dyn Fn(&WindowEvent)>) {
        let keys = Rc::new(RefCell::new(HashSet::<KeyCode>::new()));
        let focused = callback.clone();
        let held = keys.clone();
        let signal = self.0.window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                held.borrow_mut().clear();
            }
            focused(&WindowEvent::Focused(window.is_active()));
        });
        self.signal("", &self.0.window, signal);
        let controller = gtk::EventControllerKey::new();
        let pressed = keys.clone();
        let press = callback.clone();
        let signal = controller.connect_key_pressed(move |_, key, _, _| {
            let key = key_code(key);
            let repeat = !pressed.borrow_mut().insert(key.clone());
            press(&WindowEvent::KeyboardInput {
                key,
                pressed: true,
                repeat,
                synthetic: false,
            });
            gtk::glib::Propagation::Proceed
        });
        self.signal("", &controller, signal);
        let signal = controller.connect_key_released(move |_, key, _, _| {
            let key = key_code(key);
            keys.borrow_mut().remove(&key);
            callback(&WindowEvent::KeyboardInput {
                key,
                pressed: false,
                repeat: false,
                synthetic: false,
            });
        });
        self.signal("", &controller, signal);
        self.0.window.add_controller(controller);
    }

    pub fn request_quit(&self) {
        self.0.window.close();
    }

    pub fn surface(&self, parent: &Parent, id: &str, source: Binding<Option<ImageFrame>>) {
        let viewport = gtk::DrawingArea::new();
        viewport.set_hexpand(true);
        viewport.set_vexpand(true);
        viewport.set_draw_func(|_, context, _, _| {
            context.set_source_rgb(0.0, 0.0, 0.0);
            let _ = context.paint();
        });
        let weak = Rc::downgrade(&self.0);
        viewport.connect_resize(move |_, width, height| {
            if let Some(inner) = weak.upgrade() {
                let size = (width.max(0) as f32, height.max(0) as f32);
                inner.surface_size.set(size);
                let callback = inner.surface_viewport.borrow().clone();
                if let Some(callback) = callback {
                    callback(size.0, size.1);
                }
            }
        });
        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&viewport));
        overlay.add_overlay(&picture);
        self.mount_native(parent, id, &overlay);
        self.bind_native(id, source, &picture, |picture, frame| {
            let texture = frame.map(|frame| {
                gtk::gdk::MemoryTexture::new(
                    frame.width() as i32,
                    frame.height() as i32,
                    gtk::gdk::MemoryFormat::R8g8b8a8,
                    &gtk::glib::Bytes::from_owned(frame.pixels().clone()),
                    frame.width() as usize * 4,
                )
            });
            picture.set_paintable(texture.as_ref());
        });
    }
}
