//! Toolkit-independent aux vocabulary: ids, buttons, modifiers, events.
//! Both `native` and `tao_map` depend inward on this leaf.

/// Opaque native window identity for registry routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeWindowId(u64);

impl NativeWindowId {
    pub fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Test-only id from a reserved high range; every call mints a fresh
    /// value so multi-window routing stays testable (no dummy collisions).
    pub fn dummy() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(u64::MAX - 1_000_000);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

pub use wakui_core::aux::AuxId;

/// Domain mouse button: only mapped buttons cross the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuxButton {
    Left,
    Right,
    Middle,
}

/// Modifier selector for [`AuxModifiers::with_flag`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierFlag {
    Shift,
    Control,
    Alt,
    Logo,
}

/// Domain modifiers: plain data, no toolkit import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AuxModifiers {
    shift: bool,
    ctrl: bool,
    alt: bool,
    logo: bool,
}

impl AuxModifiers {
    /// No modifier held.
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn shift(&self) -> bool {
        self.shift
    }

    pub fn ctrl(&self) -> bool {
        self.ctrl
    }

    pub fn alt(&self) -> bool {
        self.alt
    }

    pub fn logo(&self) -> bool {
        self.logo
    }

    /// Set one modifier flag; chainable.
    pub fn with_flag(mut self, flag: ModifierFlag) -> Self {
        match flag {
            ModifierFlag::Shift => self.shift = true,
            ModifierFlag::Control => self.ctrl = true,
            ModifierFlag::Alt => self.alt = true,
            ModifierFlag::Logo => self.logo = true,
        }
        self
    }
}

/// Domain-level aux event: plain data, no tao/iced import.
#[derive(Debug, Clone, PartialEq)]
pub enum AuxNativeEvent {
    Focused(bool),
    CloseRequested,
    Destroyed,
    Moved(i32, i32),
    Resized(u32, u32),
    ScaleChanged(f32),
    ModifiersChanged(AuxModifiers),
    CursorMoved(f64, f64),
    CursorLeft,
    MousePressed(AuxButton),
    MouseReleased(AuxButton),
    WheelLines(f32, f32),
    WheelPixels(f32, f32),
    KeyPressed {
        key: wakui_core::Key,
        text: Option<String>,
        repeat: bool,
    },
    KeyReleased {
        key: wakui_core::Key,
    },
    Other,
}
