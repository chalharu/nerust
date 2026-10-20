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
/// Enum shape: the side is structural, never a raw discriminant byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlId {
    Digital(DigitalControlId),
    Analog(AnalogControlId),
}

impl ControlId {
    /// Digital side constructor.
    pub const fn digital(id: DigitalControlId) -> Self {
        Self::Digital(id)
    }

    /// Analog side constructor.
    pub const fn analog(id: AnalogControlId) -> Self {
        Self::Analog(id)
    }

    /// Raw discriminant plus static id string for maps and logs.
    pub const fn as_parts(self) -> (u8, &'static str) {
        match self {
            Self::Digital(id) => (0, id.as_str()),
            Self::Analog(id) => (1, id.as_str()),
        }
    }

    /// Static id string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Digital(id) => id.as_str(),
            Self::Analog(id) => id.as_str(),
        }
    }

    /// Digital side, when constructed via [`ControlId::digital`].
    pub fn as_digital(self) -> Option<DigitalControlId> {
        match self {
            Self::Digital(id) => Some(id),
            Self::Analog(_) => None,
        }
    }

    /// Analog side, when constructed via [`ControlId::analog`].
    pub fn as_analog(self) -> Option<AnalogControlId> {
        match self {
            Self::Analog(id) => Some(id),
            Self::Digital(_) => None,
        }
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

#[cfg(test)]
mod tests {
    use super::{
        AnalogControlId, AttachmentId, ControlId, DeviceKindId, DigitalControlId, PortId, ProfileId,
    };

    #[test]
    fn static_ids_round_trip_through_display_and_from() {
        let port = PortId::new("test.port");
        assert_eq!(port.as_str(), "test.port");
        assert_eq!(port.to_string(), "test.port");
        assert_eq!(PortId::from("test.port"), port);
        let as_str: &str = port.into();
        assert_eq!(as_str, "test.port");
        let attach = AttachmentId::new("test.pad");
        assert_eq!(attach.to_string(), "test.pad");
        let back: &'static str = attach.into();
        let kind = DeviceKindId::from("test.dev");
        assert_eq!(kind.as_str(), "test.dev");
        let profile = ProfileId::new("test.profile");
        assert_eq!(profile.to_string(), "test.profile");
        let analog = AnalogControlId::new("test.x");
        assert_eq!(analog.as_str(), "test.x");
        let digital = DigitalControlId::from("test.a");
        assert_eq!(digital.as_str(), "test.a");
        assert_eq!(digital.to_string(), "test.a");
        assert_eq!(back, "test.pad");
    }

    #[test]
    fn control_id_sides_are_structural() {
        let digital = DigitalControlId::new("test.a");
        // Enum shape: sides match structurally, never via a byte tag.
        assert!(matches!(ControlId::digital(digital), ControlId::Digital(_)));
        assert!(matches!(
            ControlId::analog(AnalogControlId::new("test.x")),
            ControlId::Analog(_)
        ));
    }

    #[test]
    fn control_id_sides_stay_exclusive() {
        let digital = DigitalControlId::new("test.a");
        let analog = AnalogControlId::new("test.x");
        let from_digital = ControlId::digital(digital);
        assert_eq!(from_digital.as_digital(), Some(digital));
        assert_eq!(from_digital.as_analog(), None);
        assert_eq!(from_digital.as_parts(), (0, "test.a"));
        assert_eq!(from_digital.as_str(), "test.a");
        let from_analog = ControlId::analog(analog);
        assert_eq!(from_analog.as_analog(), Some(analog));
        assert_eq!(from_analog.as_digital(), None);
        assert_eq!(from_analog, ControlId::from(analog));
        assert_eq!(ControlId::from(digital), from_digital);
        assert_eq!(ControlId::from(digital).as_analog(), None);
    }
}
