//! Named editor commands. Names are what users write in their Koto config.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
    PageUp,
    PageDown,
}

impl Motion {
    pub const ALL: [Motion; 12] = [
        Motion::Left,
        Motion::Right,
        Motion::Up,
        Motion::Down,
        Motion::WordLeft,
        Motion::WordRight,
        Motion::LineStart,
        Motion::LineEnd,
        Motion::DocStart,
        Motion::DocEnd,
        Motion::PageUp,
        Motion::PageDown,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Motion::Left => "left",
            Motion::Right => "right",
            Motion::Up => "up",
            Motion::Down => "down",
            Motion::WordLeft => "word_left",
            Motion::WordRight => "word_right",
            Motion::LineStart => "line_start",
            Motion::LineEnd => "line_end",
            Motion::DocStart => "doc_start",
            Motion::DocEnd => "doc_end",
            Motion::PageUp => "page_up",
            Motion::PageDown => "page_down",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    /// `move_<motion>`: move the cursor, collapsing any selection.
    Move(Motion),
    /// `select_<motion>`: move the cursor, extending the selection.
    Select(Motion),
    InsertNewline,
    InsertTab,
    Outdent,
    Backspace,
    Delete,
    DeleteWordLeft,
    DeleteWordRight,
    DeleteLine,
    DuplicateLine,
    MoveLineUp,
    MoveLineDown,
    Copy,
    Cut,
    Paste,
    SelectAll,
    Undo,
    Redo,
    Save,
    SaveAs,
    Quit,
    Find,
    FindNext,
    FindPrev,
    GotoLine,
    ReloadConfig,
    /// Collapse the selection.
    Cancel,
}

const SIMPLE: &[(&str, Command)] = &[
    ("insert_newline", Command::InsertNewline),
    ("insert_tab", Command::InsertTab),
    ("outdent", Command::Outdent),
    ("backspace", Command::Backspace),
    ("delete", Command::Delete),
    ("delete_word_left", Command::DeleteWordLeft),
    ("delete_word_right", Command::DeleteWordRight),
    ("delete_line", Command::DeleteLine),
    ("duplicate_line", Command::DuplicateLine),
    ("move_line_up", Command::MoveLineUp),
    ("move_line_down", Command::MoveLineDown),
    ("copy", Command::Copy),
    ("cut", Command::Cut),
    ("paste", Command::Paste),
    ("select_all", Command::SelectAll),
    ("undo", Command::Undo),
    ("redo", Command::Redo),
    ("save", Command::Save),
    ("save_as", Command::SaveAs),
    ("quit", Command::Quit),
    ("find", Command::Find),
    ("find_next", Command::FindNext),
    ("find_prev", Command::FindPrev),
    ("goto_line", Command::GotoLine),
    ("reload_config", Command::ReloadConfig),
    ("cancel", Command::Cancel),
];

impl Command {
    pub fn from_name(name: &str) -> Option<Command> {
        if let Some((_, c)) = SIMPLE.iter().find(|(n, _)| *n == name) {
            return Some(*c);
        }
        if let Some(rest) = name.strip_prefix("move_") {
            return Motion::ALL.iter().find(|m| m.name() == rest).map(|m| Command::Move(*m));
        }
        if let Some(rest) = name.strip_prefix("select_") {
            if rest == "all" {
                return Some(Command::SelectAll);
            }
            return Motion::ALL.iter().find(|m| m.name() == rest).map(|m| Command::Select(*m));
        }
        None
    }

    pub fn name(self) -> String {
        match self {
            Command::Move(m) => format!("move_{}", m.name()),
            Command::Select(m) => format!("select_{}", m.name()),
            other => SIMPLE
                .iter()
                .find(|(_, c)| *c == other)
                .map(|(n, _)| (*n).to_string())
                .unwrap_or_default(),
        }
    }

    /// Every command name, for documentation and error messages.
    pub fn all_names() -> Vec<String> {
        let mut v: Vec<String> = SIMPLE.iter().map(|(n, _)| (*n).to_string()).collect();
        for m in Motion::ALL {
            v.push(Command::Move(m).name());
            v.push(Command::Select(m).name());
        }
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_roundtrip() {
        for n in Command::all_names() {
            let c = Command::from_name(&n).unwrap_or_else(|| panic!("no command {n}"));
            assert_eq!(c.name(), n);
        }
        assert_eq!(Command::from_name("select_all"), Some(Command::SelectAll));
        assert_eq!(Command::from_name("bogus"), None);
    }
}
