//! Typed Kotlin bridge codec: v1 JSON strings into frontend events.
//! JNI glue stays in `android::menu`; this module is host-testable.

use nerust_input_traits::{GamepadAxis, StandardButton};

/// Menu button presses from the Kotlin overflow menu.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MenuAction {
    ControllerInput {
        device_id: i32,
        button: StandardButton,
        pressed: bool,
    },
    AnalogInput {
        device_id: i32,
        axis: GamepadAxis,
        value: f32,
    },
    DeviceAdded {
        device_id: i32,
        descriptor: String,
    },
    DeviceRemoved {
        device_id: i32,
    },
    CreateSlot,
    DeleteSlot(u64),
    Exit,
    LoadState,
    OpenRom,
    OpenSettings,
    OrientationChanged,
    Reset,
    SaveState,
    SelectSlot(u64),
    TogglePause,
    TouchGate {
        respond: bool,
    },
    Unload,
}

/// Single Kotlin→Rust event channel: menu presses and dialog dismiss.
/// `Eq` and `Copy` are absent: analog values ride `f32`, and device
/// descriptors ride `String`; compared by tests exactly but never hashed
/// or keyed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FrontendEvent {
    Menu(MenuAction),
    ErrorDialogDismissed(crate::error_dialog_controller::DialogRequestId),
}

use crate::bridge_contract::{
    ACTION_ANALOG, ACTION_CONTROLLER, ACTION_CREATE_SLOT, ACTION_DELETE_SLOT, ACTION_DEVICE_ADDED,
    ACTION_DEVICE_REMOVED, ACTION_EXIT, ACTION_LOAD_STATE, ACTION_OPEN_ROM, ACTION_OPEN_SETTINGS,
    ACTION_ORIENTATION_CHANGED, ACTION_RESET, ACTION_SAVE_STATE, ACTION_SELECT_SLOT,
    ACTION_TOGGLE_PAUSE, ACTION_TOUCH_GATE, ACTION_UNLOAD, ANALOG_VALUE_BOUNDS,
    BRIDGE_EVENT_VERSION, INTEGER_FIELD_BOUNDS, MENU_REQUIRED_FIELDS,
};

/// Typed controller-key mapping, checked against the shared JSON schema.
/// Single source: the decoder reads this table, no parallel match.
/// Names are bridge ids (stable for compat); positions resolve in Rust so
/// one OS keycode set serves every vendor layout.
pub(crate) const CONTROLLER_KEY_TABLE: &[(&str, StandardButton)] = &[
    ("button1", StandardButton::South),
    ("button2", StandardButton::East),
    ("button3", StandardButton::West),
    ("button4", StandardButton::North),
    ("button5", StandardButton::LeftShoulder),
    ("button6", StandardButton::RightShoulder),
    ("start", StandardButton::Start),
    ("select", StandardButton::Back),
    ("guide", StandardButton::Guide),
    ("up", StandardButton::DpadUp),
    ("down", StandardButton::DpadDown),
    ("left", StandardButton::DpadLeft),
    ("right", StandardButton::DpadRight),
    ("l3", StandardButton::LeftStick),
    ("r3", StandardButton::RightStick),
];

/// One drawer-submenu row: slot id plus its save instant. Plain data so
/// the builder stays host-testable without persistence types; the JNI
/// query maps summaries into this.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SlotListEntry {
    pub id: u64,
    pub saved_at: std::time::SystemTime,
}

/// Slot list for the drawer submenu: saved ids with epoch-millis stamps
/// plus the active id. Pure builder (host-testable); the JNI query in
/// `android::menu` only feeds it the vault mirror. Title/time/active copy
/// renders in Kotlin (app locale) from this; Rust ships ids and instants.
/// A pre-epoch stamp rides as null so Kotlin shows its unknown label
/// instead of guessing.
pub(crate) fn build_slot_list_json(slots: &[SlotListEntry], active: Option<u64>) -> String {
    let rows: Vec<serde_json::Value> = slots
        .iter()
        .map(|slot| {
            let saved_at_ms = slot
                .saved_at
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|age| age.as_millis() as i64);
            serde_json::json!({"id": slot.id, "saved_at_ms": saved_at_ms})
        })
        .collect();
    serde_json::json!({"slots": rows, "active": active}).to_string()
}

/// Versioned bridge event envelope: v1 JSON only; anything else is `None`.
/// Explicit optional fields (never flattened): `verify-contract.py`
/// parses this enum textually as a schema-parity gate.
/// Single parse: integral floats (`7.0`) decode through the flexible
/// int helpers so no `Value` round-trip normalizes before dispatch.
/// `deny_unknown_fields` keeps the schema's `additionalProperties: false`
/// fail-closed without a separate validator pass.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BridgeEventV1 {
    Menu(MenuEvent),
    ErrorDialogDismissed(ErrorDialogDismissedEvent),
}

/// `menu` event body: unknown JSON fields fail via `deny_unknown_fields`
/// (the schema's `additionalProperties: false`), integral floats decode
/// through the helpers below so no `Value` round-trip normalizes first.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MenuEvent {
    #[serde(deserialize_with = "de_integral_u32")]
    v: u32,
    action: String,
    #[serde(default, deserialize_with = "de_opt_i32")]
    device_id: Option<i32>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    pressed: Option<bool>,
    #[serde(default)]
    axis: Option<String>,
    #[serde(default)]
    value: Option<f32>,
    #[serde(default)]
    descriptor: Option<String>,
    #[serde(default, deserialize_with = "de_opt_i64")]
    slot_id: Option<i64>,
}

/// Dialog-dismiss body with the same unknown-field and integral rules.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorDialogDismissedEvent {
    #[serde(deserialize_with = "de_integral_u32")]
    v: u32,
    #[serde(deserialize_with = "de_request_id")]
    request_id: crate::error_dialog_controller::DialogRequestId,
}

/// Flexible integral decoding: JSON Schema integers include integral
/// floats (`1.0`), which plain `u32`/`i32` rejects. Accepts i64/u64/f64
/// with zero fraction and range-checks into the target.
fn de_integral_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Error, Unexpected, Visitor};
    struct V;
    impl Visitor<'_> for V {
        type Value = u32;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an integral u32")
        }
        fn visit_i64<E: Error>(self, v: i64) -> Result<u32, E> {
            u32::try_from(v).map_err(|_| Error::invalid_value(Unexpected::Signed(v), &self))
        }
        fn visit_u64<E: Error>(self, v: u64) -> Result<u32, E> {
            u32::try_from(v).map_err(|_| Error::invalid_value(Unexpected::Unsigned(v), &self))
        }
        fn visit_f64<E: Error>(self, v: f64) -> Result<u32, E> {
            if v.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&v) {
                Ok(v as u32)
            } else {
                Err(Error::invalid_value(Unexpected::Float(v), &self))
            }
        }
    }
    deserializer.deserialize_any(V)
}

/// Optional variant of the integral decoding above: missing/null rides
/// `default`/`None`; present numbers range-check into the target.
fn de_opt_i32<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Error, Unexpected, Visitor};
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<i32>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an optional integral i32")
        }
        fn visit_none<E: Error>(self) -> Result<Option<i32>, E> {
            Ok(None)
        }
        fn visit_unit<E: Error>(self) -> Result<Option<i32>, E> {
            Ok(None)
        }
        fn visit_some<D2>(self, deserializer: D2) -> Result<Option<i32>, D2::Error>
        where
            D2: serde::Deserializer<'de>,
        {
            de_integral_inner(deserializer).map(Some)
        }
        fn visit_i64<E: Error>(self, v: i64) -> Result<Option<i32>, E> {
            i32::try_from(v)
                .map(Some)
                .map_err(|_| Error::invalid_value(Unexpected::Signed(v), &self))
        }
        fn visit_u64<E: Error>(self, v: u64) -> Result<Option<i32>, E> {
            i32::try_from(v)
                .map(Some)
                .map_err(|_| Error::invalid_value(Unexpected::Unsigned(v), &self))
        }
        fn visit_f64<E: Error>(self, v: f64) -> Result<Option<i32>, E> {
            if v.fract() == 0.0 && (i32::MIN as f64..=(i32::MAX as f64)).contains(&v) {
                Ok(Some(v as i32))
            } else {
                Err(Error::invalid_value(Unexpected::Float(v), &self))
            }
        }
    }
    deserializer.deserialize_option(V)
}

/// Optional integral i64 twin for slot ids.
fn de_opt_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Error, Unexpected, Visitor};
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<i64>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an optional integral i64")
        }
        fn visit_none<E: Error>(self) -> Result<Option<i64>, E> {
            Ok(None)
        }
        fn visit_unit<E: Error>(self) -> Result<Option<i64>, E> {
            Ok(None)
        }
        fn visit_some<D2>(self, deserializer: D2) -> Result<Option<i64>, D2::Error>
        where
            D2: serde::Deserializer<'de>,
        {
            de_integral_inner(deserializer).map(Some)
        }
        fn visit_i64<E: Error>(self, v: i64) -> Result<Option<i64>, E> {
            Ok(Some(v))
        }
        fn visit_u64<E: Error>(self, v: u64) -> Result<Option<i64>, E> {
            i64::try_from(v)
                .map(Some)
                .map_err(|_| Error::invalid_value(Unexpected::Unsigned(v), &self))
        }
        fn visit_f64<E: Error>(self, v: f64) -> Result<Option<i64>, E> {
            if v.fract() == 0.0 && (i64::MIN as f64..=i64::MAX as f64).contains(&v) {
                Ok(Some(v as i64))
            } else {
                Err(Error::invalid_value(Unexpected::Float(v), &self))
            }
        }
    }
    deserializer.deserialize_option(V)
}

/// Shared inner for the `visit_some` path: present numbers only.
fn de_integral_inner<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: TryFrom<i64> + TryFrom<u64>,
    <T as TryFrom<i64>>::Error: std::fmt::Debug,
    <T as TryFrom<u64>>::Error: std::fmt::Debug,
{
    use serde::Deserialize as _;
    use serde::de::{Error, Unexpected};
    let value = serde_json::Value::deserialize(deserializer)?;
    if let Some(v) = value.as_i64() {
        return T::try_from(v)
            .map_err(|_| Error::invalid_value(Unexpected::Signed(v), &"integral target range"));
    }
    if let Some(v) = value.as_u64() {
        return T::try_from(v)
            .map_err(|_| Error::invalid_value(Unexpected::Unsigned(v), &"integral target range"));
    }
    let v = value.as_f64().ok_or_else(|| {
        Error::invalid_value(Unexpected::Other("non-numeric"), &"integral number")
    })?;
    if v.fract() != 0.0 {
        return Err(Error::invalid_value(
            Unexpected::Float(v),
            &"integral number",
        ));
    }
    // Range-check through i64/u64 before narrowing: f64 precision stops
    // at 2^53, so values past i64::MAX cannot round-trip exactly.
    if !(i64::MIN as f64..=i64::MAX as f64).contains(&v) {
        return Err(Error::invalid_value(
            Unexpected::Float(v),
            &"integral target range",
        ));
    }
    let as_i64 = v as i64;
    if (as_i64 as f64) == v {
        return T::try_from(as_i64)
            .map_err(|_| Error::invalid_value(Unexpected::Float(v), &"integral target range"));
    }
    let as_u64 = v as u64;
    T::try_from(as_u64)
        .map_err(|_| Error::invalid_value(Unexpected::Float(v), &"integral target range"))
}

/// Dialog request id with the same integral-float tolerance (syntax
/// only: the `>= 1` schema bound checks at dispatch through the table).
fn de_request_id<'de, D>(
    deserializer: D,
) -> Result<crate::error_dialog_controller::DialogRequestId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Error, Unexpected, Visitor};
    struct V;
    impl Visitor<'_> for V {
        type Value = crate::error_dialog_controller::DialogRequestId;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an integral request id")
        }
        fn visit_i64<E: Error>(
            self,
            v: i64,
        ) -> Result<crate::error_dialog_controller::DialogRequestId, E> {
            u64::try_from(v)
                .map(crate::error_dialog_controller::DialogRequestId)
                .map_err(|_| Error::invalid_value(Unexpected::Signed(v), &self))
        }
        fn visit_u64<E: Error>(
            self,
            v: u64,
        ) -> Result<crate::error_dialog_controller::DialogRequestId, E> {
            Ok(crate::error_dialog_controller::DialogRequestId(v))
        }
        fn visit_f64<E: Error>(
            self,
            v: f64,
        ) -> Result<crate::error_dialog_controller::DialogRequestId, E> {
            if v.fract() == 0.0 && (0.0..=u64::MAX as f64).contains(&v) {
                Ok(crate::error_dialog_controller::DialogRequestId(v as u64))
            } else {
                Err(Error::invalid_value(Unexpected::Float(v), &self))
            }
        }
    }
    deserializer.deserialize_any(V)
}

/// Schema-driven shape checks on one parsed menu event: required fields
/// per action ([`MENU_REQUIRED_FIELDS`]) plus integral bounds
/// ([`INTEGER_FIELD_BOUNDS`]). Rejects with a warn before typed
/// dispatch, so a gate or bound the dispatch ignores fails loud instead
/// of silently widening. Field syntax (integral floats, nulls) already
/// settled at parse; this is semantics.
fn check_menu_shape(action: &str, event: &MenuEvent) -> Option<()> {
    let required = MENU_REQUIRED_FIELDS
        .iter()
        .copied()
        .find(|(name, _)| *name == action)
        .map(|(_, fields)| fields)
        .unwrap_or(&[]);
    for field in required {
        if !event.has(field) {
            log::warn!("ignoring {action} bridge event with missing {field}");
            return None;
        }
    }
    for (field, minimum, maximum) in INTEGER_FIELD_BOUNDS.iter().copied() {
        if let Some(value) = event.get_int(field)
            && !(minimum..=maximum).contains(&value)
        {
            log::warn!("ignoring {action} bridge event with out-of-range {field}={value}");
            return None;
        }
    }
    Some(())
}

impl MenuEvent {
    /// Presence of one schema field, for the required-field table.
    fn has(&self, field: &str) -> bool {
        match field {
            "device_id" => self.device_id.is_some(),
            "key" => self.key.is_some(),
            "pressed" => self.pressed.is_some(),
            "axis" => self.axis.is_some(),
            "value" => self.value.is_some(),
            "descriptor" => self.descriptor.is_some(),
            "slot_id" => self.slot_id.is_some(),
            _ => false,
        }
    }

    /// Integral value of one schema field, for the bounds table.
    fn get_int(&self, field: &str) -> Option<i64> {
        match field {
            "device_id" => self.device_id.map(i64::from),
            "slot_id" => self.slot_id,
            _ => None,
        }
    }
}

fn decode_json_event(raw: &str) -> Option<FrontendEvent> {
    // Single typed parse: integral floats decode above, unknown fields
    // fail via `deny_unknown_fields`, ranges and required fields fail at
    // dispatch with a warn. No `Value` round-trip per input event.
    let envelope: BridgeEventV1 = serde_json::from_str(raw).ok()?;
    let version = match &envelope {
        BridgeEventV1::Menu(event) => event.v,
        BridgeEventV1::ErrorDialogDismissed(event) => event.v,
    };
    if version != BRIDGE_EVENT_VERSION {
        log::warn!("ignoring Android bridge event with unsupported version {version}");
        return None;
    }
    match envelope {
        BridgeEventV1::ErrorDialogDismissed(event) => {
            // Schema bounds through the generated table: syntax settled
            // at parse, so this is the single semantic check.
            let (_, minimum, maximum) = INTEGER_FIELD_BOUNDS
                .iter()
                .find(|(name, _, _)| *name == "request_id")
                .copied()
                .unwrap_or(("request_id", 1, i64::MAX));
            let id = event.request_id.0;
            if id >= minimum as u64 && (id as u128) <= maximum as u128 {
                Some(FrontendEvent::ErrorDialogDismissed(event.request_id))
            } else {
                log::warn!("ignoring dialog dismiss with out-of-range request id");
                None
            }
        }
        BridgeEventV1::Menu(event) => {
            check_menu_shape(&event.action, &event)?;
            decode_json_menu_action(&event)
        }
    }
}

/// Slot id for select/delete: conversion only (presence and range
/// pre-checked by the schema table). Unconvertible stays `None`.
fn decode_slot_id(slot_id: Option<i64>) -> Option<u64> {
    slot_id.and_then(|id| u64::try_from(id).ok())
}

fn decode_json_menu_action(event: &MenuEvent) -> Option<FrontendEvent> {
    // Presence and integral bounds pre-checked by the schema table;
    // the lets below bind the values (the `else` arms are unreachable
    // defense, never a second policy).
    let action = event.action.as_str();
    let key = event.key.as_deref();
    let axis = event.axis.as_deref();
    let descriptor = event.descriptor.as_deref();
    if action == ACTION_CONTROLLER {
        let (Some(device_id), Some(key), Some(pressed)) = (event.device_id, key, event.pressed)
        else {
            return None;
        };
        return decode_controller_parts(device_id, key, pressed).map(FrontendEvent::Menu);
    }
    if action == ACTION_ANALOG {
        let (Some(device_id), Some(axis), Some(value)) = (event.device_id, axis, event.value)
        else {
            return None;
        };
        return decode_analog_parts(device_id, axis, value).map(FrontendEvent::Menu);
    }
    if action == ACTION_DEVICE_ADDED {
        let (Some(device_id), Some(descriptor)) = (event.device_id, descriptor) else {
            return None;
        };
        if descriptor.is_empty() {
            log::warn!("ignoring device bridge event with missing fields");
            return None;
        }
        return Some(FrontendEvent::Menu(MenuAction::DeviceAdded {
            device_id,
            descriptor: descriptor.to_string(),
        }));
    }
    if action == ACTION_DEVICE_REMOVED {
        let Some(device_id) = event.device_id else {
            return None;
        };
        return Some(FrontendEvent::Menu(MenuAction::DeviceRemoved { device_id }));
    }
    if action == ACTION_TOUCH_GATE {
        let Some(respond) = event.pressed else {
            return None;
        };
        return Some(FrontendEvent::Menu(MenuAction::TouchGate { respond }));
    }
    match action {
        ACTION_CREATE_SLOT => Some(FrontendEvent::Menu(MenuAction::CreateSlot)),
        ACTION_DELETE_SLOT => decode_slot_id(event.slot_id)
            .map(|slot_id| FrontendEvent::Menu(MenuAction::DeleteSlot(slot_id))),
        ACTION_SELECT_SLOT => decode_slot_id(event.slot_id)
            .map(|slot_id| FrontendEvent::Menu(MenuAction::SelectSlot(slot_id))),
        ACTION_EXIT => Some(FrontendEvent::Menu(MenuAction::Exit)),
        ACTION_LOAD_STATE => Some(FrontendEvent::Menu(MenuAction::LoadState)),
        ACTION_OPEN_ROM => Some(FrontendEvent::Menu(MenuAction::OpenRom)),
        ACTION_OPEN_SETTINGS => Some(FrontendEvent::Menu(MenuAction::OpenSettings)),
        ACTION_ORIENTATION_CHANGED => Some(FrontendEvent::Menu(MenuAction::OrientationChanged)),
        ACTION_RESET => Some(FrontendEvent::Menu(MenuAction::Reset)),
        ACTION_SAVE_STATE => Some(FrontendEvent::Menu(MenuAction::SaveState)),
        ACTION_TOGGLE_PAUSE => Some(FrontendEvent::Menu(MenuAction::TogglePause)),
        ACTION_UNLOAD => Some(FrontendEvent::Menu(MenuAction::Unload)),
        unknown => {
            log::warn!("ignoring unknown Android menu action: {unknown}");
            None
        }
    }
}

fn decode_controller_parts(device_id: i32, key: &str, pressed: bool) -> Option<MenuAction> {
    let Some(button) = controller_button(key) else {
        log::warn!("ignoring unknown Android controller key: {key}");
        return None;
    };
    Some(MenuAction::ControllerInput {
        device_id,
        button,
        pressed,
    })
}

fn decode_analog_parts(device_id: i32, axis: &str, value: f32) -> Option<MenuAction> {
    let Some(axis) = controller_axis(axis) else {
        log::warn!("ignoring unknown Android controller axis: {axis}");
        return None;
    };
    if !(ANALOG_VALUE_BOUNDS.0..=ANALOG_VALUE_BOUNDS.1).contains(&value) {
        log::warn!("ignoring out-of-range Android axis value: {value}");
        return None;
    };
    Some(MenuAction::AnalogInput {
        device_id,
        axis,
        value,
    })
}

/// Typed controller-axis mapping: SDL-style names to normalized axes.
/// Single source: the decoder reads this table, no parallel match.
pub(crate) const CONTROLLER_AXIS_TABLE: &[(&str, GamepadAxis)] = &[
    ("left_x", GamepadAxis::LeftStickX),
    ("left_y", GamepadAxis::LeftStickY),
    ("right_x", GamepadAxis::RightStickX),
    ("right_y", GamepadAxis::RightStickY),
    ("trigger_l", GamepadAxis::LeftTrigger),
    ("trigger_r", GamepadAxis::RightTrigger),
];

/// Single-table lookups; the schema test iterates the tables above.
fn controller_button(key: &str) -> Option<StandardButton> {
    CONTROLLER_KEY_TABLE
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, button)| *button)
}

fn controller_axis(axis: &str) -> Option<GamepadAxis> {
    CONTROLLER_AXIS_TABLE
        .iter()
        .find(|(name, _)| *name == axis)
        .map(|(_, value)| *value)
}

/// Decode one raw bridge string: only v1 JSON decodes, never a silent drop.
pub(crate) fn decode_frontend_event(raw: &str) -> Option<FrontendEvent> {
    if !raw.trim_start().starts_with('{') {
        log::warn!("ignoring non-JSON Android bridge event");
        return None;
    }
    decode_json_event(raw)
}

#[cfg(test)]
mod tests {
    use super::{
        ACTION_ANALOG, ACTION_CREATE_SLOT, ACTION_DELETE_SLOT, ACTION_DEVICE_ADDED,
        ACTION_DEVICE_REMOVED, ACTION_EXIT, ACTION_LOAD_STATE, ACTION_OPEN_ROM,
        ACTION_OPEN_SETTINGS, ACTION_ORIENTATION_CHANGED, ACTION_RESET, ACTION_SAVE_STATE,
        ACTION_SELECT_SLOT, ACTION_TOGGLE_PAUSE, ACTION_TOUCH_GATE, ACTION_UNLOAD, FrontendEvent,
        MenuAction, decode_frontend_event,
    };
    use crate::bridge_contract::BRIDGE_EVENT_VERSION;

    #[test]
    fn decodes_every_menu_action_from_kotlin_envelope() {
        for (action, expected) in [
            (ACTION_CREATE_SLOT, MenuAction::CreateSlot),
            (ACTION_EXIT, MenuAction::Exit),
            (ACTION_LOAD_STATE, MenuAction::LoadState),
            (ACTION_OPEN_ROM, MenuAction::OpenRom),
            (ACTION_OPEN_SETTINGS, MenuAction::OpenSettings),
            (ACTION_ORIENTATION_CHANGED, MenuAction::OrientationChanged),
            (ACTION_RESET, MenuAction::Reset),
            (ACTION_SAVE_STATE, MenuAction::SaveState),
            (ACTION_TOGGLE_PAUSE, MenuAction::TogglePause),
            (ACTION_UNLOAD, MenuAction::Unload),
        ] {
            let raw =
                format!(r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"{action}"}}"#);
            assert_eq!(
                decode_frontend_event(&raw),
                Some(FrontendEvent::Menu(expected)),
                "action {action}"
            );
        }
        // The touch gate carries its respond flag in `pressed`; a missing
        // flag fails loud instead of guessing a default.
        for respond in [true, false] {
            let raw = format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"{ACTION_TOUCH_GATE}","pressed":{respond}}}"#
            );
            assert_eq!(
                decode_frontend_event(&raw),
                Some(FrontendEvent::Menu(MenuAction::TouchGate { respond })),
                "touch gate {respond}"
            );
        }
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"touch_gate"}}"#
            )),
            None,
            "touch gate without pressed never defaults"
        );
    }

    #[test]
    fn decode_json_event_maps_versioned_menu() {
        // open_rom is covered by decodes_every_menu_action_from_kotlin_envelope;
        // this pins the gated controller shape and the second envelope type.
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"controller","device_id":7,"key":"button1","pressed":true}}"#
            )),
            Some(FrontendEvent::Menu(MenuAction::ControllerInput {
                device_id: 7,
                button: nerust_input_traits::StandardButton::South,
                pressed: true,
            }))
        );
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"error_dialog_dismissed","request_id":7}}"#
            )),
            Some(FrontendEvent::ErrorDialogDismissed(
                crate::error_dialog_controller::DialogRequestId(7)
            ))
        );
    }

    #[test]
    fn slot_list_json_ships_ids_instants_and_active() {
        use std::time::{Duration, UNIX_EPOCH};

        use super::{SlotListEntry, build_slot_list_json};

        fn entry(id: u64, secs: u64) -> SlotListEntry {
            SlotListEntry {
                id,
                saved_at: UNIX_EPOCH + Duration::from_secs(secs),
            }
        }

        let raw = build_slot_list_json(&[entry(2, 1_700_000_001), entry(1, 1_700_000_000)], None);
        let value: serde_json::Value = serde_json::from_str(&raw).expect("slot list is JSON");
        assert_eq!(value["slots"][0]["id"], 2);
        assert_eq!(value["slots"][0]["saved_at_ms"], 1_700_000_001_000_i64);
        assert!(value["active"].is_null());
        let raw = build_slot_list_json(&[entry(1, 1_700_000_000)], Some(1));
        let value: serde_json::Value = serde_json::from_str(&raw).expect("slot list is JSON");
        assert_eq!(value["active"], 1);
    }

    #[test]
    fn slot_actions_round_trip_and_reject_bad_ids() {
        for (action, expected) in [
            (ACTION_SELECT_SLOT, MenuAction::SelectSlot(3)),
            (ACTION_DELETE_SLOT, MenuAction::DeleteSlot(3)),
        ] {
            let raw = format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"{action}","slot_id":3}}"#
            );
            assert_eq!(
                decode_frontend_event(&raw),
                Some(FrontendEvent::Menu(expected)),
                "action {action}"
            );
        }
        for raw in [
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"{ACTION_SELECT_SLOT}"}}"#
            ),
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"{ACTION_DELETE_SLOT}","slot_id":-1}}"#
            ),
        ] {
            assert_eq!(
                decode_frontend_event(&raw),
                None,
                "slot action without a usable id never defaults"
            );
        }
    }

    #[test]
    fn decode_json_event_rejects_bad_version_and_unknown() {
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{},"type":"menu","action":"open_rom"}}"#,
                BRIDGE_EVENT_VERSION + 1
            )),
            None
        );
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"mystery"}}"#
            )),
            None
        );
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"controller","device_id":7}}"#
            )),
            None
        );
        // Bare action names and colon-separated payloads never reach the
        // typed dispatch: the JSON prefix guard rejects them first.
        for legacy in [
            ACTION_OPEN_ROM,
            "controller:7:button1:1",
            "error_dialog_dismissed",
        ] {
            assert_eq!(decode_frontend_event(legacy), None);
        }
        // Unknown actions inside a valid envelope reach the typed dispatch
        // and fail there (not at the prefix guard).
        for unknown in [
            ACTION_OPEN_SETTINGS,
            ACTION_TOGGLE_PAUSE,
            ACTION_SAVE_STATE,
            ACTION_LOAD_STATE,
            ACTION_RESET,
            ACTION_EXIT,
            ACTION_UNLOAD,
            ACTION_ORIENTATION_CHANGED,
        ] {
            assert_eq!(
                decode_frontend_event(&format!(
                    r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"legacy_{unknown}"}}"#
                )),
                None
            );
        }
    }

    /// Contract pin: the JSON schema and the Rust decoder share one version
    /// and one action table; a one-side drift fails loudly.
    #[test]
    fn bridge_schema_matches_versioned_contract() {
        let doc: serde_json::Value = serde_json::from_str(include_str!("../bridge_events.v1.json"))
            .expect("bridge schema must parse");
        assert_eq!(
            doc["definitions"]["menuEvent"]["properties"]["v"]["const"],
            BRIDGE_EVENT_VERSION
        );
        assert_eq!(
            doc["definitions"]["errorDialogDismissedEvent"]["properties"]["v"]["const"],
            BRIDGE_EVENT_VERSION
        );
        assert_eq!(
            doc["x-settings-schema-version"],
            crate::bridge_contract::SETTINGS_SCHEMA_VERSION
        );
        assert_eq!(
            doc["x-saf-chunk-bytes"],
            crate::bridge_contract::SAF_CHUNK_BYTES
        );
        let actions = doc["definitions"]["menuEvent"]["properties"]["action"]["enum"]
            .as_array()
            .expect("action enum must list every menu action");
        let names: Vec<&str> = actions
            .iter()
            .map(|value| value.as_str().expect("action enum entries are strings"))
            .collect();
        assert_eq!(
            names.len(),
            17,
            "new actions must also be handled by the decoder"
        );
        for action in [
            ACTION_ANALOG,
            ACTION_CREATE_SLOT,
            ACTION_DELETE_SLOT,
            ACTION_DEVICE_ADDED,
            ACTION_DEVICE_REMOVED,
            ACTION_EXIT,
            ACTION_LOAD_STATE,
            ACTION_OPEN_ROM,
            ACTION_OPEN_SETTINGS,
            ACTION_ORIENTATION_CHANGED,
            ACTION_RESET,
            ACTION_SAVE_STATE,
            ACTION_SELECT_SLOT,
            ACTION_TOGGLE_PAUSE,
            ACTION_TOUCH_GATE,
            ACTION_UNLOAD,
            "controller",
        ] {
            assert!(names.contains(&action), "schema is missing action {action}");
        }
    }

    /// Device connect/disconnect round-trips with descriptors; empty
    /// descriptors and missing ids fail loud instead of registering junk.
    #[test]
    fn device_events_carry_descriptors() {
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"device_added","device_id":7,"descriptor":"93c16c27e6378d0"}}"#
            )),
            Some(FrontendEvent::Menu(MenuAction::DeviceAdded {
                device_id: 7,
                descriptor: "93c16c27e6378d0".to_string(),
            }))
        );
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"device_removed","device_id":7}}"#
            )),
            Some(FrontendEvent::Menu(MenuAction::DeviceRemoved {
                device_id: 7
            }))
        );
        for raw in [
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"device_added","device_id":7}}"#
            ),
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"device_added","device_id":7,"descriptor":""}}"#
            ),
            format!(r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"device_removed"}}"#),
        ] {
            assert_eq!(decode_frontend_event(&raw), None);
        }
    }

    /// Axis parity: the decoder table and the schema axis enum list
    /// exactly the same axes, and every one round-trips with a float.
    #[test]
    fn controller_axis_table_matches_schema() {
        use super::{CONTROLLER_AXIS_TABLE, decode_frontend_event};
        use nerust_input_traits::GamepadAxis;
        let doc: serde_json::Value = serde_json::from_str(include_str!("../bridge_events.v1.json"))
            .expect("bridge schema must parse");
        let axes = doc["definitions"]["menuEvent"]["properties"]["axis"]["enum"]
            .as_array()
            .expect("axis enum must list every controller axis");
        let schema_axes: Vec<&str> = axes
            .iter()
            .map(|value| value.as_str().expect("axis enum entries are strings"))
            .collect();
        assert_eq!(schema_axes.len(), CONTROLLER_AXIS_TABLE.len());
        for (name, axis) in CONTROLLER_AXIS_TABLE {
            assert!(
                schema_axes.contains(name),
                "schema is missing controller axis {name}"
            );
            let raw = format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"analog","device_id":3,"axis":"{name}","value":0.5}}"#
            );
            assert_eq!(
                decode_frontend_event(&raw),
                Some(FrontendEvent::Menu(MenuAction::AnalogInput {
                    device_id: 3,
                    axis: *axis,
                    value: 0.5,
                })),
                "axis {name} must round-trip"
            );
        }
        for raw in [
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"analog","device_id":3,"axis":"mystery","value":0.5}}"#
            ),
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"analog","device_id":3,"axis":"left_x","value":1.5}}"#
            ),
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"analog","device_id":3,"axis":"left_x"}}"#
            ),
        ] {
            assert_eq!(decode_frontend_event(&raw), None);
        }
        assert_eq!(
            GamepadAxis::LeftStickX,
            CONTROLLER_AXIS_TABLE[0].1,
            "axis order stays SDL-canonical"
        );
    }
    /// Key parity: the decoder table and the schema key enum list
    /// exactly the same keys; Kotlin `KEY_*` pins the same set in CI.
    #[test]
    fn controller_key_table_matches_schema() {
        use super::{CONTROLLER_KEY_TABLE, decode_frontend_event};
        let doc: serde_json::Value = serde_json::from_str(include_str!("../bridge_events.v1.json"))
            .expect("bridge schema must parse");
        let keys = doc["definitions"]["menuEvent"]["properties"]["key"]["enum"]
            .as_array()
            .expect("key enum must list every controller key");
        let schema_keys: Vec<&str> = keys
            .iter()
            .map(|value| value.as_str().expect("key enum entries are strings"))
            .collect();
        assert_eq!(schema_keys.len(), CONTROLLER_KEY_TABLE.len());
        for (name, button) in CONTROLLER_KEY_TABLE {
            assert!(
                schema_keys.contains(name),
                "schema is missing controller key {name}"
            );
            let raw = format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"controller","device_id":3,"key":"{name}","pressed":true}}"#
            );
            assert_eq!(
                decode_frontend_event(&raw),
                Some(FrontendEvent::Menu(MenuAction::ControllerInput {
                    device_id: 3,
                    button: *button,
                    pressed: true,
                })),
                "key {name} must round-trip"
            );
        }
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"controller","device_id":3,"key":"mystery","pressed":true}}"#
            )),
            None
        );
    }
}
