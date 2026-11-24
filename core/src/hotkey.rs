//! Keyboard shortcuts as written in the settings file: `ctrl+alt+s`,
//! `ctrl+shift+f9`, `win+alt+,`. Platform code maps `Key` to its own key
//! codes. A shortcut needs at least one modifier, so a plain letter can never
//! be stolen from every app.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A letter (stored upper case), digit, or one of `, . / ; - = [ ] '`.
    Char(char),
    /// F1 to F24.
    F(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

impl Mods {
    pub fn any(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.win
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub mods: Mods,
    pub key: Key,
}

const PUNCTUATION: &str = ",./;-=[]'";

impl Key {
    fn parse(text: &str) -> Option<Key> {
        let t = text.trim();
        let lower = t.to_ascii_lowercase();
        if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
            return (1..=24).contains(&n).then_some(Key::F(n));
        }
        let named = match lower.as_str() {
            "comma" => Some(','),
            "period" | "dot" => Some('.'),
            "slash" => Some('/'),
            "semicolon" => Some(';'),
            "minus" => Some('-'),
            "equals" | "plus" => Some('='),
            _ => None,
        };
        if let Some(c) = named {
            return Some(Key::Char(c));
        }
        let mut chars = t.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_alphanumeric() => Some(Key::Char(c.to_ascii_uppercase())),
            (Some(c), None) if PUNCTUATION.contains(c) => Some(Key::Char(c)),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Key::Char(c) => c.to_string(),
            Key::F(n) => format!("F{n}"),
        }
    }
}

impl Hotkey {
    /// `None` for an empty string (shortcut turned off) or anything invalid.
    pub fn parse(text: &str) -> Option<Hotkey> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        // Split on '+', but "ctrl+alt++" style is not supported; use "plus".
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let (key_part, mod_parts) = parts.split_last()?;
        let mut mods = Mods::default();
        for m in mod_parts {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods.ctrl = true,
                "alt" => mods.alt = true,
                "shift" => mods.shift = true,
                "win" | "windows" | "super" | "meta" => mods.win = true,
                _ => return None,
            }
        }
        let key = Key::parse(key_part)?;
        mods.any().then_some(Hotkey { mods, key })
    }

    /// The settings-file form: `ctrl+alt+s`.
    pub fn key_text(&self) -> String {
        let mut out = String::new();
        for (on, name) in
            [(self.mods.ctrl, "ctrl"), (self.mods.alt, "alt"), (self.mods.shift, "shift"), (self.mods.win, "win")]
        {
            if on {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(&self.key.label().to_ascii_lowercase());
        out
    }

    /// For people: `Ctrl+Alt+S`.
    pub fn label(&self) -> String {
        let mut out = String::new();
        for (on, name) in
            [(self.mods.ctrl, "Ctrl"), (self.mods.alt, "Alt"), (self.mods.shift, "Shift"), (self.mods.win, "Win")]
        {
            if on {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(&self.key.label());
        out
    }
}

/// What each shortcut does. The order is the order shown in Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    Dashboard,
    Readings,
    Settings,
    Pin,
    Line,
}

impl Command {
    pub const ALL: [Command; 5] =
        [Command::Dashboard, Command::Readings, Command::Settings, Command::Pin, Command::Line];

    pub fn key(self) -> &'static str {
        match self {
            Command::Dashboard => "hotkey-dashboard",
            Command::Readings => "hotkey-readings",
            Command::Settings => "hotkey-settings",
            Command::Pin => "hotkey-pin",
            Command::Line => "hotkey-line",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Command::Dashboard => "Open or close the dashboard",
            Command::Readings => "Show or hide the readings",
            Command::Settings => "Open settings",
            Command::Pin => "Pin or unpin the readings",
            Command::Line => "Show or hide the line",
        }
    }

    pub fn default_text(self) -> &'static str {
        match self {
            Command::Dashboard => "ctrl+alt+s",
            Command::Readings => "ctrl+alt+r",
            Command::Settings => "ctrl+alt+,",
            Command::Pin => "ctrl+alt+p",
            Command::Line => "ctrl+alt+h",
        }
    }

    pub fn index(self) -> usize {
        Command::ALL.iter().position(|&c| c == self).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats() {
        let h = Hotkey::parse("Ctrl+Alt+s").unwrap();
        assert_eq!(h.key, Key::Char('S'));
        assert!(h.mods.ctrl && h.mods.alt && !h.mods.shift);
        assert_eq!(h.key_text(), "ctrl+alt+s");
        assert_eq!(h.label(), "Ctrl+Alt+S");
        let f = Hotkey::parse("shift + win + F9").unwrap();
        assert_eq!(f.key, Key::F(9));
        assert_eq!(f.label(), "Shift+Win+F9");
        assert_eq!(Hotkey::parse("ctrl+alt+,").unwrap().key, Key::Char(','));
        assert_eq!(Hotkey::parse("ctrl+alt+comma").unwrap().key_text(), "ctrl+alt+,");
    }

    #[test]
    fn rejects_bad_shortcuts() {
        assert_eq!(Hotkey::parse(""), None);
        assert_eq!(Hotkey::parse("s"), None, "no modifier");
        assert_eq!(Hotkey::parse("ctrl+alt"), None);
        assert_eq!(Hotkey::parse("hyper+s"), None);
        assert_eq!(Hotkey::parse("ctrl+f99"), None);
        assert_eq!(Hotkey::parse("ctrl+ab"), None);
    }

    #[test]
    fn defaults_are_valid_and_distinct() {
        let all: Vec<Hotkey> = Command::ALL.iter().map(|c| Hotkey::parse(c.default_text()).unwrap()).collect();
        for (i, a) in all.iter().enumerate() {
            assert!(all[i + 1..].iter().all(|b| b != a));
        }
    }
}
