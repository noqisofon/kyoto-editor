//! crossterm -> kyoto-core key conversion.

use crossterm::event::{KeyCode as CtCode, KeyEvent, KeyModifiers};
use kyoto_core::{Key, KeyCode, Modifiers};

pub fn convert(ev: KeyEvent) -> Option<Key> {
    let mut mods = Modifiers {
        ctrl: ev.modifiers.contains(KeyModifiers::CONTROL),
        alt: ev.modifiers.contains(KeyModifiers::ALT),
        shift: ev.modifiers.contains(KeyModifiers::SHIFT),
    };
    let code = match ev.code {
        CtCode::Char(c) => KeyCode::Char(c),
        CtCode::Enter => KeyCode::Enter,
        CtCode::Tab => KeyCode::Tab,
        CtCode::BackTab => {
            mods.shift = true;
            KeyCode::Tab
        }
        CtCode::Backspace => KeyCode::Backspace,
        CtCode::Delete => KeyCode::Delete,
        CtCode::Esc => KeyCode::Esc,
        CtCode::Left => KeyCode::Left,
        CtCode::Right => KeyCode::Right,
        CtCode::Up => KeyCode::Up,
        CtCode::Down => KeyCode::Down,
        CtCode::Home => KeyCode::Home,
        CtCode::End => KeyCode::End,
        CtCode::PageUp => KeyCode::PageUp,
        CtCode::PageDown => KeyCode::PageDown,
        CtCode::Insert => KeyCode::Insert,
        CtCode::F(n) => KeyCode::F(n),
        _ => return None,
    };
    Some(Key::new(code, mods))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backtab_is_shift_tab() {
        let k = convert(KeyEvent::new(CtCode::BackTab, KeyModifiers::SHIFT)).unwrap();
        assert_eq!(k, Key::new(KeyCode::Tab, Modifiers::SHIFT));
    }

    #[test]
    fn ctrl_shift_letter() {
        let k = convert(KeyEvent::new(CtCode::Char('Z'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)).unwrap();
        assert_eq!(k.normalized(), Key::parse("ctrl+shift+z").unwrap());
    }
}
