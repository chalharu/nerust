//! Static string identifiers for ports, attachments, devices, and controls.

/// Define a `&'static str` newtype with `new`/`as_str`/`Display`/`From`.
/// Distinct types so mismatched kinds fail at compile time.
macro_rules! static_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(&'static str);

        impl $name {
            pub const fn new(value: &'static str) -> Self {
                Self(value)
            }

            pub const fn as_str(self) -> &'static str {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }

        impl From<&'static str> for $name {
            fn from(value: &'static str) -> Self {
                Self(value)
            }
        }

        impl From<$name> for &'static str {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

static_id!(PortId);
static_id!(AttachmentId);
static_id!(DeviceKindId);
static_id!(DigitalControlId);
static_id!(ProfileId);
static_id!(AnalogControlId);

/// Unified control identifier across digital and analog controls.
/// Opaque wrapper: construct via `From`/`From` for each side, match via accessors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlId(u8, &'static str);

impl ControlId {
    /// Digital side constructor.
    pub const fn digital(id: DigitalControlId) -> Self {
        Self(0, id.as_str())
    }

    /// Analog side constructor.
    pub const fn analog(id: AnalogControlId) -> Self {
        Self(1, id.as_str())
    }

    /// Raw discriminant plus static id string for maps and logs.
    pub const fn as_parts(self) -> (u8, &'static str) {
        (self.0, self.1)
    }

    /// Static id string.
    pub const fn as_str(self) -> &'static str {
        self.1
    }

    /// Digital side, when constructed via [`ControlId::digital`].
    pub fn as_digital(self) -> Option<DigitalControlId> {
        (self.0 == 0).then(|| DigitalControlId::new(self.1))
    }

    /// Analog side, when constructed via [`ControlId::analog`].
    pub fn as_analog(self) -> Option<AnalogControlId> {
        (self.0 == 1).then(|| AnalogControlId::new(self.1))
    }
}

impl From<DigitalControlId> for ControlId {
    fn from(id: DigitalControlId) -> Self {
        Self::digital(id)
    }
}

impl From<AnalogControlId> for ControlId {
    fn from(id: AnalogControlId) -> Self {
        Self::analog(id)
    }
}
