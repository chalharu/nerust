//! Parse `db.txt` (SDL_GameControllerDB line format) into a static table.
//! Only the current target's `platform:` entries are emitted; unknown
//! platforms compile to an empty table (Phase 3 fallback: unknown pad).

use std::{env, fs, path::Path};

/// Target OS to SDL platform tag. Unlisted targets get no entries.
fn platform_tag() -> Option<&'static str> {
    match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("linux") => Some("Linux"),
        Ok("windows") => Some("Windows"),
        Ok("macos") => Some("Mac OS X"),
        Ok("android") => Some("Android"),
        Ok("ios") => Some("iOS"),
        _ => None,
    }
}

/// SDL logical button name to position-based variant path.
fn button_path(name: &str) -> Option<&'static str> {
    match name {
        "a" => Some("StandardButton::South"),
        "b" => Some("StandardButton::East"),
        "x" => Some("StandardButton::West"),
        "y" => Some("StandardButton::North"),
        "back" => Some("StandardButton::Back"),
        "guide" => Some("StandardButton::Guide"),
        "start" => Some("StandardButton::Start"),
        "leftstick" => Some("StandardButton::LeftStick"),
        "rightstick" => Some("StandardButton::RightStick"),
        "leftshoulder" => Some("StandardButton::LeftShoulder"),
        "rightshoulder" => Some("StandardButton::RightShoulder"),
        "dpup" => Some("StandardButton::DpadUp"),
        "dpdown" => Some("StandardButton::DpadDown"),
        "dpleft" => Some("StandardButton::DpadLeft"),
        "dpright" => Some("StandardButton::DpadRight"),
        "misc1" => Some("StandardButton::Misc1"),
        "paddle1" => Some("StandardButton::Paddle1"),
        "paddle2" => Some("StandardButton::Paddle2"),
        "paddle3" => Some("StandardButton::Paddle3"),
        "paddle4" => Some("StandardButton::Paddle4"),
        _ => None,
    }
}

/// SDL logical axis name to variant path.
fn axis_path(name: &str) -> Option<&'static str> {
    match name {
        "leftx" => Some("GamepadAxis::LeftStickX"),
        "lefty" => Some("GamepadAxis::LeftStickY"),
        "rightx" => Some("GamepadAxis::RightStickX"),
        "righty" => Some("GamepadAxis::RightStickY"),
        "lefttrigger" => Some("GamepadAxis::LeftTrigger"),
        "righttrigger" => Some("GamepadAxis::RightTrigger"),
        _ => None,
    }
}

/// GUID hex to (vendor, product): `u16` LE pairs at bytes 4 and 8.
fn vid_pid(guid: &str) -> Option<(u16, u16)> {
    if guid.len() != 32 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..32)
        .step_by(2)
        .map(|i| u8::from_str_radix(&guid[i..i + 2], 16).ok())
        .collect();
    // 32 hex chars always collect exactly 16 bytes; a bad digit
    // short-circuits to `None` above.
    let bytes = bytes?;
    let vid = u16::from_le_bytes([bytes[4], bytes[5]]);
    let pid = u16::from_le_bytes([bytes[8], bytes[9]]);
    Some((vid, pid))
}

/// Numeric suffix after one prefix char (`b0` → `0`).
fn suffix_index(prefix: char, right: &str) -> Option<&str> {
    let index = right.strip_prefix(prefix)?;
    (!index.is_empty() && index.bytes().all(|b| b.is_ascii_digit())).then_some(index)
}

/// Hat spec (`h0.1` → `("0", "1")`).
fn hat_parts(right: &str) -> Option<(&str, &str)> {
    let (hat, mask) = right.strip_prefix('h')?.split_once('.')?;
    (!hat.is_empty()
        && !mask.is_empty()
        && hat.bytes().all(|b| b.is_ascii_digit())
        && mask.bytes().all(|b| b.is_ascii_digit()))
    .then_some((hat, mask))
}

/// Escape one name for a Rust string literal.
fn escape(name: &str) -> String {
    name.replace('\\', "\\\\").replace('"', "\\\"")
}

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    println!("cargo:rerun-if-changed={}", dir.join("db.txt").display());
    let text = fs::read_to_string(dir.join("db.txt")).expect("gamepad-db: db.txt missing");
    let want = platform_tag();
    let mut out = String::from("pub(crate) static MAPPINGS: &[RawMapping] = &[\n");
    if let Some(want) = want {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split(',');
            let guid = parts.next().unwrap_or("");
            let name = parts.next().unwrap_or("");
            let Some((vid, pid)) = vid_pid(guid) else {
                continue;
            };
            let mut platform = "";
            let mut buttons = String::new();
            let mut axes = String::new();
            let mut hats = String::new();
            for token in parts {
                if let Some(tag) = token.strip_prefix("platform:") {
                    platform = tag;
                    continue;
                }
                let Some((left, right)) = token.split_once(':') else {
                    continue;
                };
                if let Some(path) = button_path(left) {
                    if let Some(index) = suffix_index('b', right) {
                        buttons.push_str(&format!("({path}, {index}), "));
                    } else if let Some((hat, mask)) = hat_parts(right) {
                        hats.push_str(&format!("({path}, {hat}, {mask}), "));
                    }
                    // Button-style triggers (`lefttrigger:bN`) have no digital
                    // position: Phase 1 keeps triggers analog-only.
                } else if let Some(path) = axis_path(left)
                    && let Some(index) = suffix_index('a', right)
                {
                    axes.push_str(&format!("({path}, {index}), "));
                }
            }
            if platform != want {
                continue;
            }
            out.push_str(&format!(
                "RawMapping {{ vendor_id: {vid:#06x}, product_id: {pid:#06x}, \
                 name: \"{}\", buttons: &[{buttons}], axes: &[{axes}], hats: &[{hats}] }},\n",
                escape(name),
            ));
        }
    }
    out.push_str("];\n");
    let generated = Path::new(&env::var("OUT_DIR").expect("OUT_DIR missing")).join("mappings.rs");
    fs::write(generated, out).expect("gamepad-db: cannot write mappings.rs");
}
