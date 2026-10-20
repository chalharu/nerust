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
