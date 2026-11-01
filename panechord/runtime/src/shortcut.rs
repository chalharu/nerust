//! Portable menu shortcut grammar: modifiers first, then one ASCII letter,
//! digit, F1..F12, Comma, or editing/navigation key. Tokens are case-insensitive and
//! trimmed. Modifiers: Primary, Ctrl, Cmd, Alt, Shift (no duplicates).
//! Primary resolves to Cmd on macOS, Ctrl elsewhere. This is not a promise of
//! physical-key/layout identity: GTK uses keyvals, muda's Accelerator uses Code.
//! Backend-specific legacy syntax is deliberately NOT parsed here.
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutTarget {
    MacOS,
    Other,
}
impl ShortcutTarget {
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOS
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    modifiers: u8,
    key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutError;
impl fmt::Display for ShortcutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected [Primary|Ctrl|Cmd|Alt|Shift +] letter/digit/F1..F12/editing key")
    }
}
impl std::error::Error for ShortcutError {}

// canonical, GTK keyval name, muda Code name
const KEYS: &[(&str, &str, &str)] = &[
    ("Comma", "comma", "Comma"),
    ("Enter", "Return", "Enter"),
    ("Tab", "Tab", "Tab"),
    ("Space", "space", "Space"),
    ("Escape", "Escape", "Escape"),
    ("Backspace", "BackSpace", "Backspace"),
    ("Delete", "Delete", "Delete"),
    ("Insert", "Insert", "Insert"),
    ("Home", "Home", "Home"),
    ("End", "End", "End"),
    ("PageUp", "Page_Up", "PageUp"),
    ("PageDown", "Page_Down", "PageDown"),
    ("Left", "Left", "ArrowLeft"),
    ("Right", "Right", "ArrowRight"),
    ("Up", "Up", "ArrowUp"),
    ("Down", "Down", "ArrowDown"),
];

impl FromStr for Shortcut {
    type Err = ShortcutError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let tokens: Vec<_> = value.split('+').map(str::trim).collect();
        let mut modifiers = 0;
        for token in &tokens[..tokens.len() - 1] {
            let bit = match token.to_ascii_lowercase().as_str() {
                "primary" => 1,
                "ctrl" => 2,
                "cmd" => 4,
                "alt" => 8,
                "shift" => 16,
                _ => return Err(ShortcutError),
            };
            if modifiers & bit != 0 {
                return Err(ShortcutError);
            }
            modifiers |= bit;
        }
        // Avoid target-dependent duplicate modifiers such as Primary+Ctrl.
        if modifiers & 1 != 0 && modifiers & 6 != 0 {
            return Err(ShortcutError);
        }
        let raw = tokens[tokens.len() - 1];
        let key = if raw.len() == 1 && raw.as_bytes()[0].is_ascii_alphanumeric() {
            raw.to_ascii_uppercase()
        } else if let Some((name, _, _)) = KEYS
            .iter()
            .find(|(name, _, _)| name.eq_ignore_ascii_case(raw))
        {
            (*name).into()
        } else {
            let upper = raw.to_ascii_uppercase();
            let n = upper
                .strip_prefix('F')
                .and_then(|n| n.parse::<u8>().ok())
                .ok_or(ShortcutError)?;
            if !(1..=12).contains(&n) || upper != format!("F{n}") {
                return Err(ShortcutError);
            }
            upper
        };
        Ok(Self { modifiers, key })
    }
}

impl Shortcut {
    pub fn parse(value: &str) -> Result<Self, ShortcutError> {
        value.parse()
    }
    pub fn to_gtk(&self) -> String {
        self.to_gtk_for(ShortcutTarget::current())
    }
    pub fn to_muda(&self) -> String {
        self.to_muda_for(ShortcutTarget::current())
    }
    fn resolved(&self, target: ShortcutTarget) -> u8 {
        (self.modifiers & !1)
            | if self.modifiers & 1 == 0 {
                0
            } else if target == ShortcutTarget::MacOS {
                4
            } else {
                2
            }
    }
    pub fn to_gtk_for(&self, target: ShortcutTarget) -> String {
        let mods = self.resolved(target);
        let mut out = String::new();
        for (bit, name) in [(2, "Control"), (4, "Meta"), (8, "Alt"), (16, "Shift")] {
            if mods & bit != 0 {
                out.push_str(&format!("<{name}>"));
            }
        }
        if let Some((_, gtk, _)) = KEYS.iter().find(|(key, _, _)| *key == self.key) {
            out.push_str(gtk);
        } else if self.key.len() == 1 {
            out.push_str(&self.key.to_ascii_lowercase());
        } else {
            out.push_str(&self.key);
        }
        out
    }
    /// AppKit key equivalent for `NSMenuItem`: `(modifierFlags, key)`.
    /// Returns `None` for keys AppKit cannot express as an equivalent
    /// (navigation/editing keys); those items still work, they just show no
    /// shortcut. NSEventModifierFlags values: Shift 1<<17, Control 1<<18,
    /// Option 1<<19, Command 1<<20.
    pub fn to_appkit(&self) -> Option<(u64, String)> {
        let mut flags = 0u64;
        // Bits match FromStr: 1 Primary, 2 Ctrl, 4 Cmd, 8 Alt, 16 Shift.
        // Primary resolves to Command on macOS (rejected mixed with Ctrl/Cmd
        // at parse time), mirroring resolved().
        if self.modifiers & 1 != 0 {
            flags |= 1 << 20;
        }
        if self.modifiers & 2 != 0 {
            flags |= 1 << 18;
        }
        if self.modifiers & 4 != 0 {
            flags |= 1 << 20;
        }
        if self.modifiers & 8 != 0 {
            flags |= 1 << 19;
        }
        if self.modifiers & 16 != 0 {
            flags |= 1 << 17;
        }
        if self.modifiers & !31 != 0 {
            return None;
        }
        let key = if self.key.len() == 1 {
            self.key.to_ascii_lowercase()
        } else if let Some(n) = self
            .key
            .strip_prefix('F')
            .and_then(|n| n.parse::<u32>().ok())
            && (1..=12).contains(&n)
        {
            // NSF1FunctionKey..NSF12FunctionKey (U+F704..U+F70F).
            char::from_u32(0xF704 + n - 1)?.to_string()
        } else {
            match self.key.as_str() {
                "Comma" => ",".into(),
                "Enter" => "\r".into(),
                "Tab" => "\t".into(),
                "Space" => " ".into(),
                "Escape" => "\u{1b}".into(),
                "Backspace" => "\u{7f}".into(),
                "Delete" => "\u{F728}".into(),
                "Left" => "\u{F702}".into(),
                "Right" => "\u{F703}".into(),
                "Up" => "\u{F700}".into(),
                "Down" => "\u{F701}".into(),
                _ => return None,
            }
        };
        Some((flags, key))
    }
    pub fn to_muda_for(&self, target: ShortcutTarget) -> String {
        let mods = self.resolved(target);
        let mut parts = Vec::new();
        for (bit, name) in [(2, "Ctrl"), (4, "Cmd"), (8, "Alt"), (16, "Shift")] {
            if mods & bit != 0 {
                parts.push(name.to_owned());
            }
        }
        parts.push(
            if let Some((_, _, muda)) = KEYS.iter().find(|(key, _, _)| *key == self.key) {
                (*muda).into()
            } else if self.key.len() == 1 && self.key.as_bytes()[0].is_ascii_alphabetic() {
                format!("Key{}", self.key)
            } else if self.key.len() == 1 {
                format!("Digit{}", self.key)
            } else {
                self.key.clone()
            },
        );
        parts.join("+")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn primary_resolves_for_target_not_build_host() {
        for key in ["S", "Q"] {
            let shortcut = Shortcut::parse(&format!("Primary+{key}")).unwrap();
            assert_eq!(
                shortcut.to_gtk_for(ShortcutTarget::MacOS),
                format!("<Meta>{}", key.to_lowercase())
            );
            assert_eq!(
                shortcut.to_muda_for(ShortcutTarget::MacOS),
                format!("Cmd+Key{key}")
            );
            assert_eq!(
                shortcut.to_gtk_for(ShortcutTarget::Other),
                format!("<Control>{}", key.to_lowercase())
            );
            assert_eq!(
                shortcut.to_muda_for(ShortcutTarget::Other),
                format!("Ctrl+Key{key}")
            );
        }
    }
    #[test]
    fn preferences_shortcut_resolves_comma_for_both_targets() {
        let shortcut = Shortcut::parse("Primary+Comma").unwrap();
        assert_eq!(shortcut.to_gtk_for(ShortcutTarget::MacOS), "<Meta>comma");
        assert_eq!(shortcut.to_gtk_for(ShortcutTarget::Other), "<Control>comma");
        assert_eq!(shortcut.to_muda_for(ShortcutTarget::MacOS), "Cmd+Comma");
        assert_eq!(shortcut.to_muda_for(ShortcutTarget::Other), "Ctrl+Comma");
    }
    #[test]
    fn appkit_equivalent_maps_primary_to_command() {
        let shortcut = Shortcut::parse("Primary+O").unwrap();
        assert_eq!(shortcut.to_appkit(), Some(((1 << 20), "o".into())));
        let shortcut = Shortcut::parse("Primary+Shift+S").unwrap();
        assert_eq!(
            shortcut.to_appkit(),
            Some(((1 << 20) | (1 << 17), "s".into()))
        );
        let shortcut = Shortcut::parse("Primary+Comma").unwrap();
        assert_eq!(shortcut.to_appkit(), Some(((1 << 20), ",".into())));
        let shortcut = Shortcut::parse("Primary+F5").unwrap();
        assert_eq!(shortcut.to_appkit().map(|(flags, _)| flags), Some(1 << 20));
        assert!(
            shortcut
                .to_appkit()
                .is_some_and(|(_, key)| key.chars().count() == 1)
        );
        assert_eq!(Shortcut::parse("Primary+Home").unwrap().to_appkit(), None);
    }
    #[test]
    fn modifiers_are_normalized_without_implicit_shift() {
        let shortcut = Shortcut::parse(" shift + CTRL + s ").unwrap();
        assert_eq!(shortcut.to_gtk(), "<Control><Shift>s");
        assert_eq!(shortcut.to_muda(), "Ctrl+Shift+KeyS");
        assert_eq!(Shortcut::parse("S"), Shortcut::parse("s"));
        assert_eq!(
            Shortcut::parse("Cmd+Alt+9").unwrap().to_muda(),
            "Cmd+Alt+Digit9"
        );
    }
    #[test]
    fn supported_keys_convert() {
        for (name, gtk, muda) in KEYS {
            let shortcut = Shortcut::parse(name).unwrap();
            assert_eq!(shortcut.to_gtk(), *gtk);
            assert_eq!(shortcut.to_muda(), *muda);
        }
        for n in 1..=12 {
            let key = format!("F{n}");
            assert_eq!(Shortcut::parse(&key).unwrap().to_gtk(), key);
        }
        for key in '0'..='9' {
            assert!(Shortcut::parse(&key.to_string()).is_ok());
        }
        for key in 'A'..='Z' {
            assert!(Shortcut::parse(&key.to_string()).is_ok());
        }
    }
    #[test]
    fn malformed_and_legacy_are_not_common_grammar() {
        for input in [
            "",
            "+",
            "Ctrl+",
            "Ctrl++S",
            "Ctrl+Ctrl+S",
            "Primary+Cmd+S",
            "Primary+Ctrl+S",
            "S+Ctrl",
            "Ctrl+S+Q",
            "F0",
            "F13",
            "F01",
            "é",
            "Ctrl+KeyS",
            "<Control>s",
        ] {
            assert!(Shortcut::parse(input).is_err(), "{input}");
        }
    }
}
