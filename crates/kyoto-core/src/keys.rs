//! Backend-independent key types, key string parsing and the key map.

use std::collections::HashMap;
use std::fmt;

use crate::command::{Command, Motion};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Esc,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    F(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Modifiers {
    pub const NONE: Self = Self { ctrl: false, alt: false, shift: false };
    pub const CTRL: Self = Self { ctrl: true, alt: false, shift: false };
    pub const SHIFT: Self = Self { ctrl: false, alt: false, shift: true };
    pub const ALT: Self = Self { ctrl: false, alt: true, shift: false };
    pub const CTRL_SHIFT: Self = Self { ctrl: true, alt: false, shift: true };
}

/// A key press as received from a terminal backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub mods: Modifiers,
}

impl Key {
    pub const fn new(code: KeyCode, mods: Modifiers) -> Self {
        Self { code, mods }
    }

    /// The character this key inserts when it is not bound to anything.
    pub fn text_char(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if !self.mods.ctrl && !self.mods.alt && !c.is_control() => Some(c),
            _ => None,
        }
    }

    /// Canonical form used for map lookups: letters are lower-cased (an
    /// upper-case letter implies shift) and shift is dropped on symbols,
    /// since the symbol itself already encodes it.
    pub fn normalized(self) -> Self {
        let mut k = self;
        if let KeyCode::Char(c) = k.code {
            if c.is_alphabetic() {
                let lower: Vec<char> = c.to_lowercase().collect();
                if lower.len() == 1 && lower[0] != c {
                    k.code = KeyCode::Char(lower[0]);
                    k.mods.shift = true;
                }
            } else {
                k.mods.shift = false;
            }
        }
        k
    }

    /// Parses strings such as `ctrl+shift+z`, `alt+up`, `f5`, `ctrl++`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty key string".into());
        }
        let (mod_part, key_part) = match s.rfind('+') {
            None => ("", s),
            Some(0) => ("", "+"),
            Some(i) if i == s.len() - 1 => (s[..i].strip_suffix('+').unwrap_or(&s[..i]), "+"),
            Some(i) => (&s[..i], &s[i + 1..]),
        };
        let mut mods = Modifiers::NONE;
        for m in mod_part.split('+').filter(|m| !m.is_empty()) {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "c" => mods.ctrl = true,
                "alt" | "meta" | "option" | "a" => mods.alt = true,
                "shift" | "s" => mods.shift = true,
                other => return Err(format!("unknown modifier `{other}` in `{s}`")),
            }
        }
        let lower = key_part.to_ascii_lowercase();
        let code = match lower.as_str() {
            "enter" | "return" => KeyCode::Enter,
            "tab" => KeyCode::Tab,
            "backspace" | "bs" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "esc" | "escape" => KeyCode::Esc,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" | "pgdown" => KeyCode::PageDown,
            "insert" | "ins" => KeyCode::Insert,
            "space" => KeyCode::Char(' '),
            "plus" => KeyCode::Char('+'),
            f if f.len() >= 2 && f.starts_with('f') && f[1..].chars().all(|c| c.is_ascii_digit()) => {
                let n: u8 = f[1..].parse().map_err(|_| format!("bad function key `{s}`"))?;
                if !(1..=24).contains(&n) {
                    return Err(format!("function key out of range in `{s}`"));
                }
                KeyCode::F(n)
            }
            _ => {
                let mut it = key_part.chars();
                match (it.next(), it.next()) {
                    (Some(c), None) => KeyCode::Char(c),
                    _ => return Err(format!("unknown key `{key_part}` in `{s}`")),
                }
            }
        };
        Ok(Key::new(code, mods).normalized())
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.ctrl {
            f.write_str("ctrl+")?;
        }
        if self.mods.alt {
            f.write_str("alt+")?;
        }
        if self.mods.shift {
            f.write_str("shift+")?;
        }
        match self.code {
            KeyCode::Char(' ') => f.write_str("space"),
            KeyCode::Char(c) => write!(f, "{c}"),
            KeyCode::F(n) => write!(f, "f{n}"),
            other => write!(f, "{}", format!("{other:?}").to_lowercase()),
        }
    }
}

/// What a key is bound to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    /// A built-in command.
    Command(Command),
    /// A function defined in the user's Koto config (index into the script).
    Script(usize),
}

#[derive(Clone, Debug)]
pub struct KeyMap {
    map: HashMap<Key, Binding>,
}

impl KeyMap {
    pub fn empty() -> Self {
        Self { map: HashMap::new() }
    }

    pub fn bind(&mut self, key: Key, binding: Binding) {
        self.map.insert(key.normalized(), binding);
    }

    pub fn unbind(&mut self, key: Key) {
        self.map.remove(&key.normalized());
    }

    pub fn lookup(&self, key: Key) -> Option<Binding> {
        self.map.get(&key.normalized()).copied()
    }

    /// All bindings, sorted by key string (for help screens and tests).
    pub fn bindings(&self) -> Vec<(String, Binding)> {
        let mut v: Vec<_> = self.map.iter().map(|(k, b)| (k.to_string(), *b)).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// The default CUA ("Common User Access") key bindings: the familiar
    /// Ctrl+C/X/V/Z/Y/A/S, Shift+arrow selection, Ctrl+arrow word motion.
    pub fn cua() -> Self {
        use Command::*;
        use KeyCode as K;
        use Motion as M;
        let mut km = Self::empty();
        let mut b = |code: KeyCode, mods: Modifiers, cmd: Command| {
            km.bind(Key::new(code, mods), Binding::Command(cmd));
        };
        let none = Modifiers::NONE;
        let ctrl = Modifiers::CTRL;
        let shift = Modifiers::SHIFT;
        let cs = Modifiers::CTRL_SHIFT;
        let alt = Modifiers::ALT;

        // Cursor movement and Shift-selection.
        for (code, motion) in [
            (K::Left, M::Left),
            (K::Right, M::Right),
            (K::Up, M::Up),
            (K::Down, M::Down),
            (K::Home, M::LineStart),
            (K::End, M::LineEnd),
            (K::PageUp, M::PageUp),
            (K::PageDown, M::PageDown),
        ] {
            b(code, none, Move(motion));
            b(code, shift, Select(motion));
        }
        for (code, motion) in [
            (K::Left, M::WordLeft),
            (K::Right, M::WordRight),
            (K::Home, M::DocStart),
            (K::End, M::DocEnd),
        ] {
            b(code, ctrl, Move(motion));
            b(code, cs, Select(motion));
        }

        // Editing.
        b(K::Enter, none, InsertNewline);
        b(K::Enter, shift, InsertNewline);
        b(K::Tab, none, InsertTab);
        b(K::Tab, shift, Outdent);
        b(K::Backspace, none, Backspace);
        b(K::Backspace, shift, Backspace);
        b(K::Delete, none, Delete);
        b(K::Backspace, ctrl, DeleteWordLeft);
        b(K::Delete, ctrl, DeleteWordRight);
        b(K::Char('k'), cs, DeleteLine);
        b(K::Char('d'), cs, DuplicateLine);
        b(K::Up, alt, MoveLineUp);
        b(K::Down, alt, MoveLineDown);
        b(K::Esc, none, Cancel);

        // Clipboard, history, selection.
        b(K::Char('c'), ctrl, Copy);
        b(K::Insert, ctrl, Copy);
        b(K::Char('x'), ctrl, Cut);
        b(K::Delete, shift, Cut);
        b(K::Char('v'), ctrl, Paste);
        b(K::Insert, shift, Paste);
        b(K::Char('z'), ctrl, Undo);
        b(K::Char('y'), ctrl, Redo);
        b(K::Char('z'), cs, Redo);
        b(K::Char('a'), ctrl, SelectAll);

        // File and search.
        b(K::Char('s'), ctrl, Save);
        b(K::Char('s'), cs, SaveAs);
        b(K::Char('q'), ctrl, Quit);
        b(K::Char('f'), ctrl, Find);
        b(K::F(3), none, FindNext);
        b(K::F(3), shift, FindPrev);
        b(K::Char('g'), ctrl, GotoLine);
        km
    }
}

impl Default for KeyMap {
    fn default() -> Self {
        Self::cua()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keys() {
        assert_eq!(
            Key::parse("Ctrl+Shift+Z").unwrap(),
            Key::new(KeyCode::Char('z'), Modifiers::CTRL_SHIFT)
        );
        assert_eq!(Key::parse("ctrl++").unwrap(), Key::new(KeyCode::Char('+'), Modifiers::CTRL));
        assert_eq!(Key::parse("F5").unwrap(), Key::new(KeyCode::F(5), Modifiers::NONE));
        assert!(Key::parse("hyper+x").is_err());
        assert!(Key::parse("ctrl+nope").is_err());
    }

    #[test]
    fn uppercase_implies_shift() {
        let km = KeyMap::cua();
        let k = Key::new(KeyCode::Char('Z'), Modifiers::CTRL_SHIFT);
        assert_eq!(km.lookup(k), Some(Binding::Command(Command::Redo)));
    }

    #[test]
    fn cua_basics() {
        let km = KeyMap::cua();
        let k = |s: &str| km.lookup(Key::parse(s).unwrap());
        assert_eq!(k("ctrl+c"), Some(Binding::Command(Command::Copy)));
        assert_eq!(k("ctrl+v"), Some(Binding::Command(Command::Paste)));
        assert_eq!(k("shift+left"), Some(Binding::Command(Command::Select(Motion::Left))));
        assert_eq!(k("a"), None);
    }
}
