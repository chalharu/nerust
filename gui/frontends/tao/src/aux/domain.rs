//! Toolkit-independent aux vocabulary: ids, buttons, modifiers, events.
//! Both `native` and `tao_map` depend inward on this leaf.

/// Opaque native window identity for registry routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NativeWindowId(u64);

impl NativeWindowId {
    pub(crate) fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Test-only dummy id: distinctness across dummies is not guaranteed.
    #[cfg(test)]
    pub(crate) fn dummy() -> Self {
        Self(u64::MAX)
    }
}

/// Registry-assigned aux entry handle. Routing keys on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct AuxId(u64);

impl AuxId {
    pub(crate) fn new(raw: u64) -> Self {
        Self(raw)
    }
}

/// Domain mouse button: only mapped buttons cross the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuxButton {
    Left,
    Right,
    Middle,
}

/// Modifier selector for [`AuxModifiers::with_flag`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModifierFlag {
    Shift,
    Control,
    Alt,
    Logo,
}

/// Domain modifiers: plain data, no toolkit import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct AuxModifiers {
    shift: bool,
    ctrl: bool,
    alt: bool,
    logo: bool,
}

impl AuxModifiers {
    /// No modifier held.
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn shift(&self) -> bool {
        self.shift
    }

    pub(crate) fn ctrl(&self) -> bool {
        self.ctrl
    }

    pub(crate) fn alt(&self) -> bool {
        self.alt
    }

    pub(crate) fn logo(&self) -> bool {
        self.logo
    }

    /// Set one modifier flag; chainable.
    pub(crate) fn with_flag(mut self, flag: ModifierFlag) -> Self {
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
pub(crate) enum AuxNativeEvent {
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
        key: nerust_keyboard::Key,
        text: Option<String>,
        repeat: bool,
    },
    KeyReleased {
        key: nerust_keyboard::Key,
    },
    TextCommit(String),
    Other,
}
