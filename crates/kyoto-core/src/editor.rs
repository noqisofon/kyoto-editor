//! Editor state and command execution (cursor, selection, undo, clipboard).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use unicode_segmentation::UnicodeSegmentation;

use crate::buffer::{
    Buffer, Pos, advance, col_from_display, display_col, next_boundary, prev_boundary,
};
use crate::command::{Command, Motion};

/// Options that influence editing behaviour (a subset of the user config).
#[derive(Clone, Copy, Debug)]
pub struct EditOptions {
    pub tab_width: usize,
    pub insert_spaces: bool,
    pub auto_indent: bool,
}

impl Default for EditOptions {
    fn default() -> Self {
        Self { tab_width: 4, insert_spaces: true, auto_indent: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    Find,
    GotoLine,
    SaveAs,
}

/// Something the front end must do after a command ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
    Prompt(PromptKind),
    ReloadConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupKind {
    Typing,
    Other,
}

#[derive(Clone, Debug)]
struct Edit {
    start: Pos,
    removed: String,
    inserted: String,
}

#[derive(Clone, Copy, Debug)]
struct Sel {
    cursor: Pos,
    anchor: Option<Pos>,
}

#[derive(Clone, Debug)]
struct Group {
    edits: Vec<Edit>,
    before: Sel,
    after: Sel,
    kind: GroupKind,
}

/// Text handed to a user script as context.
#[derive(Clone, Debug)]
pub struct ScriptContext {
    pub selection: String,
    pub line: String,
    /// 1-based.
    pub line_number: usize,
    /// 1-based, in characters.
    pub column: usize,
    pub path: Option<String>,
}

const MAX_TYPING_GROUP: usize = 64;

pub struct Editor {
    buffer: Buffer,
    cursor: Pos,
    anchor: Option<Pos>,
    desired_col: Option<usize>,
    path: Option<PathBuf>,
    pub options: EditOptions,
    undo: Vec<Group>,
    redo: Vec<Group>,
    open: Option<Group>,
    save_len: usize,
    clipboard: String,
    clipboard_linewise: bool,
    clipboard_export: Option<String>,
    search: Option<String>,
    page_size: usize,
    status: Option<String>,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        Self {
            buffer: Buffer::default(),
            cursor: Pos::default(),
            anchor: None,
            desired_col: None,
            path: None,
            options: EditOptions::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            open: None,
            save_len: 0,
            clipboard: String::new(),
            clipboard_linewise: false,
            clipboard_export: None,
            search: None,
            page_size: 20,
            status: None,
        }
    }

    /// Opens `path`; a missing file yields an empty buffer bound to that path.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let mut ed = Self::new();
        match fs::read_to_string(path) {
            Ok(text) => ed.buffer = Buffer::from_text(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        ed.path = Some(path.to_path_buf());
        Ok(ed)
    }

    pub fn from_text(text: &str) -> Self {
        let mut ed = Self::new();
        ed.buffer = Buffer::from_text(text);
        ed
    }

    // ---- accessors -------------------------------------------------------

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }
    pub fn cursor(&self) -> Pos {
        self.cursor
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub fn is_modified(&self) -> bool {
        self.open.is_some() || self.undo.len() != self.save_len
    }
    pub fn set_page_size(&mut self, rows: usize) {
        self.page_size = rows.max(1);
    }
    pub fn take_status(&mut self) -> Option<String> {
        self.status.take()
    }
    /// Text of the last copy/cut, once, so a front end can forward it to the
    /// system clipboard.
    pub fn take_clipboard_export(&mut self) -> Option<String> {
        self.clipboard_export.take()
    }
    pub fn text(&self) -> String {
        self.buffer.slice(Pos::default(), self.buffer.end_pos())
    }

    /// Ordered selection range, if non-empty.
    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        if a == self.cursor {
            None
        } else {
            Some((a.min(self.cursor), a.max(self.cursor)))
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|(s, e)| self.buffer.slice(s, e))
    }

    pub fn script_context(&self) -> ScriptContext {
        let line = self.buffer.line(self.cursor.line);
        ScriptContext {
            selection: self.selected_text().unwrap_or_default(),
            line: line.to_string(),
            line_number: self.cursor.line + 1,
            column: line[..self.cursor.col].chars().count() + 1,
            path: self.path.as_ref().map(|p| p.display().to_string()),
        }
    }

    // ---- undo plumbing ---------------------------------------------------

    fn sel(&self) -> Sel {
        Sel { cursor: self.cursor, anchor: self.anchor }
    }

    fn begin(&mut self, kind: GroupKind) {
        if self.open.is_some() {
            return;
        }
        self.redo.clear();
        if kind == GroupKind::Typing {
            if let Some(last) = self.undo.last() {
                let continuous = last.kind == GroupKind::Typing
                    && last.edits.len() < MAX_TYPING_GROUP
                    && last.after.cursor == self.cursor
                    && last.after.anchor.is_none()
                    && self.anchor.is_none();
                if continuous {
                    if self.undo.len() == self.save_len {
                        self.save_len = usize::MAX;
                    }
                    self.open = self.undo.pop();
                    return;
                }
            }
        }
        self.open = Some(Group {
            edits: Vec::new(),
            before: self.sel(),
            after: self.sel(),
            kind,
        });
    }

    fn end(&mut self) {
        if let Some(mut g) = self.open.take() {
            if g.edits.is_empty() {
                return;
            }
            g.after = self.sel();
            self.undo.push(g);
        }
    }

    /// Applies one replacement inside the open group.
    fn apply(&mut self, start: Pos, end: Pos, text: &str) -> Pos {
        let removed = self.buffer.slice(start, end);
        let new_end = self.buffer.replace(start, end, text);
        if let Some(g) = self.open.as_mut() {
            g.edits.push(Edit { start, removed, inserted: text.to_string() });
        }
        new_end
    }

    pub fn undo(&mut self) {
        self.end();
        let Some(g) = self.undo.pop() else {
            self.status = Some("Nothing to undo".into());
            return;
        };
        for e in g.edits.iter().rev() {
            let end = advance(e.start, &e.inserted);
            self.buffer.replace(e.start, end, &e.removed);
        }
        self.cursor = self.buffer.clamp(g.before.cursor);
        self.anchor = g.before.anchor.map(|a| self.buffer.clamp(a));
        self.desired_col = None;
        self.redo.push(g);
    }

    pub fn redo(&mut self) {
        self.end();
        let Some(g) = self.redo.pop() else {
            self.status = Some("Nothing to redo".into());
            return;
        };
        for e in &g.edits {
            let end = advance(e.start, &e.removed);
            self.buffer.replace(e.start, end, &e.inserted);
        }
        self.cursor = self.buffer.clamp(g.after.cursor);
        self.anchor = g.after.anchor.map(|a| self.buffer.clamp(a));
        self.desired_col = None;
        self.undo.push(g);
    }

    // ---- low level editing ----------------------------------------------

    fn normalize_newlines(text: &str) -> String {
        text.replace("\r\n", "\n").replace('\r', "\n")
    }

    /// Replaces the selection (if any) with `text` and moves the cursor
    /// after it.
    fn replace_selection(&mut self, text: &str) {
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        let new_end = self.apply(start, end, text);
        self.cursor = new_end;
        self.anchor = None;
        self.desired_col = None;
    }

    /// Inserts text at the cursor, replacing the selection. Used for typed
    /// characters, terminal paste and script results.
    pub fn insert_text(&mut self, text: &str) {
        let text = Self::normalize_newlines(text);
        if text.is_empty() {
            return;
        }
        let single = text.chars().count() == 1 && text != "\n";
        self.begin(if single && self.selection().is_none() {
            GroupKind::Typing
        } else {
            GroupKind::Other
        });
        self.replace_selection(&text);
        self.end();
    }

    fn indent_unit(&self) -> String {
        if self.options.insert_spaces {
            " ".repeat(self.options.tab_width.max(1))
        } else {
            "\t".to_string()
        }
    }

    fn line_range(&self) -> (usize, usize) {
        match self.selection() {
            Some((s, e)) => {
                // A selection ending at column 0 of a line does not include it.
                let last = if e.col == 0 && e.line > s.line { e.line - 1 } else { e.line };
                (s.line, last)
            }
            None => (self.cursor.line, self.cursor.line),
        }
    }

    // ---- movement --------------------------------------------------------

    fn is_word(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    fn word_left(&self, mut p: Pos) -> Pos {
        loop {
            let line = self.buffer.line(p.line);
            // Skip whitespace backwards.
            let mut col = p.col;
            while col > 0 {
                let c = line[..col].chars().next_back().unwrap();
                if c.is_whitespace() {
                    col -= c.len_utf8();
                } else {
                    break;
                }
            }
            if col == 0 {
                if p.line == 0 {
                    return Pos::new(0, 0);
                }
                if col == p.col {
                    // Already at line start: cross into previous line.
                    p = Pos::new(p.line - 1, self.buffer.line(p.line - 1).len());
                    return p;
                }
                return Pos::new(p.line, 0);
            }
            let last = line[..col].chars().next_back().unwrap();
            let class = Self::is_word(last);
            while col > 0 {
                let c = line[..col].chars().next_back().unwrap();
                if c.is_whitespace() || Self::is_word(c) != class {
                    break;
                }
                col -= c.len_utf8();
            }
            return self.buffer.clamp(Pos::new(p.line, col));
        }
    }

    fn word_right(&self, p: Pos) -> Pos {
        let line = self.buffer.line(p.line);
        if p.col >= line.len() {
            return if p.line + 1 < self.buffer.line_count() {
                Pos::new(p.line + 1, 0)
            } else {
                p
            };
        }
        let mut col = p.col;
        let first = line[col..].chars().next().unwrap();
        if !first.is_whitespace() {
            let class = Self::is_word(first);
            while let Some(c) = line[col..].chars().next() {
                if c.is_whitespace() || Self::is_word(c) != class {
                    break;
                }
                col += c.len_utf8();
            }
        }
        while let Some(c) = line[col..].chars().next() {
            if !c.is_whitespace() {
                break;
            }
            col += c.len_utf8();
        }
        self.buffer.clamp(Pos::new(p.line, col))
    }

    fn display_col_of(&self, p: Pos) -> usize {
        display_col(self.buffer.line(p.line), p.col, self.options.tab_width)
    }

    fn move_vertical(&mut self, delta: isize) -> Pos {
        let want = match self.desired_col {
            Some(w) => w,
            None => {
                let w = self.display_col_of(self.cursor);
                self.desired_col = Some(w);
                w
            }
        };
        let last = self.buffer.line_count() as isize - 1;
        let line = (self.cursor.line as isize + delta).clamp(0, last) as usize;
        if line == self.cursor.line {
            // Already at the first/last line: jump to the line edge.
            return if delta < 0 {
                Pos::new(line, 0)
            } else {
                Pos::new(line, self.buffer.line(line).len())
            };
        }
        let col = col_from_display(self.buffer.line(line), want, self.options.tab_width);
        Pos::new(line, col)
    }

    fn target_of(&mut self, m: Motion, select: bool) -> Pos {
        let c = self.cursor;
        let line = self.buffer.line(c.line);
        // Plain Left/Right with a selection collapse to its edge (CUA).
        if !select {
            if let Some((s, e)) = self.selection() {
                match m {
                    Motion::Left => return s,
                    Motion::Right => return e,
                    _ => {}
                }
            }
        }
        match m {
            Motion::Left => {
                if c.col > 0 {
                    Pos::new(c.line, prev_boundary(line, c.col))
                } else if c.line > 0 {
                    Pos::new(c.line - 1, self.buffer.line(c.line - 1).len())
                } else {
                    c
                }
            }
            Motion::Right => {
                if c.col < line.len() {
                    Pos::new(c.line, next_boundary(line, c.col))
                } else if c.line + 1 < self.buffer.line_count() {
                    Pos::new(c.line + 1, 0)
                } else {
                    c
                }
            }
            Motion::Up => self.move_vertical(-1),
            Motion::Down => self.move_vertical(1),
            Motion::PageUp => self.move_vertical(-(self.page_size as isize)),
            Motion::PageDown => self.move_vertical(self.page_size as isize),
            Motion::WordLeft => self.word_left(c),
            Motion::WordRight => self.word_right(c),
            Motion::LineStart => {
                // Smart home: first press goes to indentation, second to column 0.
                let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
                if c.col != indent && indent != line.len() {
                    Pos::new(c.line, indent)
                } else {
                    Pos::new(c.line, 0)
                }
            }
            Motion::LineEnd => Pos::new(c.line, line.len()),
            Motion::DocStart => Pos::default(),
            Motion::DocEnd => self.buffer.end_pos(),
        }
    }

    fn do_move(&mut self, m: Motion, select: bool) {
        self.end();
        let vertical = matches!(m, Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown);
        if !vertical {
            self.desired_col = None;
        }
        let target = self.target_of(m, select);
        if select {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = target;
        if select && self.anchor == Some(self.cursor) {
            self.anchor = None;
        }
    }

    // ---- commands --------------------------------------------------------

    /// Executes a command and tells the front end whether it must act.
    pub fn execute(&mut self, cmd: Command) -> Effect {
        match cmd {
            Command::Move(m) => self.do_move(m, false),
            Command::Select(m) => self.do_move(m, true),
            Command::InsertNewline => self.newline(),
            Command::InsertTab => self.tab(),
            Command::Outdent => self.outdent(),
            Command::Backspace => self.delete_char(false),
            Command::Delete => self.delete_char(true),
            Command::DeleteWordLeft => self.delete_word(false),
            Command::DeleteWordRight => self.delete_word(true),
            Command::DeleteLine => self.delete_lines(),
            Command::DuplicateLine => self.duplicate_lines(),
            Command::MoveLineUp => self.move_lines(-1),
            Command::MoveLineDown => self.move_lines(1),
            Command::Copy => self.copy(false),
            Command::Cut => self.copy(true),
            Command::Paste => self.paste(),
            Command::SelectAll => {
                self.end();
                self.anchor = Some(Pos::default());
                self.cursor = self.buffer.end_pos();
            }
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            Command::Save => {
                return if self.path.is_some() {
                    self.save_reporting();
                    Effect::None
                } else {
                    Effect::Prompt(PromptKind::SaveAs)
                };
            }
            Command::SaveAs => return Effect::Prompt(PromptKind::SaveAs),
            Command::Quit => return Effect::Quit,
            Command::Find => return Effect::Prompt(PromptKind::Find),
            Command::FindNext => return self.find_command(true),
            Command::FindPrev => return self.find_command(false),
            Command::GotoLine => return Effect::Prompt(PromptKind::GotoLine),
            Command::ReloadConfig => return Effect::ReloadConfig,
            Command::Cancel => {
                self.end();
                self.anchor = None;
            }
        }
        Effect::None
    }

    fn newline(&mut self) {
        self.begin(GroupKind::Other);
        let (start, _) = self.selection().unwrap_or((self.cursor, self.cursor));
        let mut text = String::from("\n");
        if self.options.auto_indent {
            let line = self.buffer.line(start.line);
            let indent_len = line.len() - line.trim_start_matches([' ', '\t']).len();
            text.push_str(&line[..indent_len.min(start.col)]);
        }
        self.replace_selection(&text);
        self.end();
    }

    fn tab(&mut self) {
        if let Some((s, e)) = self.selection() {
            if s.line != e.line {
                self.indent_lines();
                return;
            }
        }
        self.begin(GroupKind::Other);
        let text = if self.options.insert_spaces {
            let x = self.display_col_of(self.selection().map_or(self.cursor, |(s, _)| s));
            let tw = self.options.tab_width.max(1);
            " ".repeat(tw - x % tw)
        } else {
            "\t".to_string()
        };
        self.replace_selection(&text);
        self.end();
    }

    fn adjust_cols(&mut self, f: impl Fn(Pos) -> Pos) {
        self.cursor = f(self.cursor);
        self.anchor = self.anchor.map(&f);
    }

    fn indent_lines(&mut self) {
        let (a, b) = self.line_range();
        let unit = self.indent_unit();
        self.begin(GroupKind::Other);
        for l in a..=b {
            if !self.buffer.line(l).is_empty() {
                self.apply(Pos::new(l, 0), Pos::new(l, 0), &unit);
            }
        }
        let n = unit.len();
        let buf = &self.buffer;
        let shift = |p: Pos| {
            if (a..=b).contains(&p.line) && !buf.line(p.line).is_empty() {
                Pos::new(p.line, p.col + n)
            } else {
                p
            }
        };
        let (c, an) = (shift(self.cursor), self.anchor.map(shift));
        self.cursor = c;
        self.anchor = an;
        self.end();
    }

    fn outdent(&mut self) {
        let (a, b) = self.line_range();
        let tw = self.options.tab_width.max(1);
        self.begin(GroupKind::Other);
        let mut removed = vec![0usize; b - a + 1];
        for l in a..=b {
            let line = self.buffer.line(l);
            let n = if line.starts_with('\t') {
                1
            } else {
                line.chars().take(tw).take_while(|c| *c == ' ').count()
            };
            if n > 0 {
                self.apply(Pos::new(l, 0), Pos::new(l, n), "");
                removed[l - a] = n;
            }
        }
        let shift = |p: Pos| {
            if (a..=b).contains(&p.line) {
                Pos::new(p.line, p.col.saturating_sub(removed[p.line - a]))
            } else {
                p
            }
        };
        self.adjust_cols(shift);
        self.end();
    }

    fn delete_char(&mut self, forward: bool) {
        if self.selection().is_some() {
            self.begin(GroupKind::Other);
            self.replace_selection("");
            self.end();
            return;
        }
        let c = self.cursor;
        let line = self.buffer.line(c.line);
        let (start, end) = if forward {
            if c.col < line.len() {
                (c, Pos::new(c.line, next_boundary(line, c.col)))
            } else if c.line + 1 < self.buffer.line_count() {
                (c, Pos::new(c.line + 1, 0))
            } else {
                return;
            }
        } else if c.col > 0 {
            (Pos::new(c.line, prev_boundary(line, c.col)), c)
        } else if c.line > 0 {
            (Pos::new(c.line - 1, self.buffer.line(c.line - 1).len()), c)
        } else {
            return;
        };
        self.begin(GroupKind::Other);
        self.apply(start, end, "");
        self.cursor = start;
        self.desired_col = None;
        self.end();
    }

    fn delete_word(&mut self, forward: bool) {
        if self.selection().is_some() {
            return self.delete_char(forward);
        }
        let c = self.cursor;
        let other = if forward { self.word_right(c) } else { self.word_left(c) };
        if other == c {
            return;
        }
        let (s, e) = (c.min(other), c.max(other));
        self.begin(GroupKind::Other);
        self.apply(s, e, "");
        self.cursor = s;
        self.desired_col = None;
        self.end();
    }

    /// Range covering whole lines `a..=b` including one adjoining newline.
    fn whole_lines(&self, a: usize, b: usize) -> (Pos, Pos) {
        if b + 1 < self.buffer.line_count() {
            (Pos::new(a, 0), Pos::new(b + 1, 0))
        } else if a > 0 {
            (Pos::new(a - 1, self.buffer.line(a - 1).len()), self.buffer.end_pos())
        } else {
            (Pos::new(0, 0), self.buffer.end_pos())
        }
    }

    fn delete_lines(&mut self) {
        let (a, b) = self.line_range();
        let (s, e) = self.whole_lines(a, b);
        self.begin(GroupKind::Other);
        self.apply(s, e, "");
        self.anchor = None;
        let line = a.min(self.buffer.line_count() - 1);
        self.cursor = self.buffer.clamp(Pos::new(line, self.cursor.col));
        self.desired_col = None;
        self.end();
    }

    fn duplicate_lines(&mut self) {
        let (a, b) = self.line_range();
        let block = self.buffer.slice(Pos::new(a, 0), Pos::new(b, self.buffer.line(b).len()));
        let at = Pos::new(b, self.buffer.line(b).len());
        self.begin(GroupKind::Other);
        self.apply(at, at, &format!("\n{block}"));
        let n = b - a + 1;
        self.cursor = Pos::new(self.cursor.line + n, self.cursor.col);
        self.anchor = self.anchor.map(|p| Pos::new(p.line + n, p.col));
        self.end();
    }

    fn move_lines(&mut self, dir: isize) {
        let (a, b) = self.line_range();
        let last = self.buffer.line_count() - 1;
        if (dir < 0 && a == 0) || (dir > 0 && b == last) {
            return;
        }
        let block = self.buffer.slice(Pos::new(a, 0), Pos::new(b, self.buffer.line(b).len()));
        let (lo, hi, new_text) = if dir < 0 {
            let above = self.buffer.line(a - 1).to_string();
            (a - 1, b, format!("{block}\n{above}"))
        } else {
            let below = self.buffer.line(b + 1).to_string();
            (a, b + 1, format!("{below}\n{block}"))
        };
        let start = Pos::new(lo, 0);
        let end = Pos::new(hi, self.buffer.line(hi).len());
        self.begin(GroupKind::Other);
        self.apply(start, end, &new_text);
        let shift = |p: Pos| Pos::new((p.line as isize + dir) as usize, p.col);
        self.adjust_cols(shift);
        self.end();
    }

    // ---- clipboard -------------------------------------------------------

    fn copy(&mut self, cut: bool) {
        self.end();
        let (text, linewise) = match self.selected_text() {
            Some(t) => (t, false),
            None => {
                let l = self.buffer.line(self.cursor.line);
                (format!("{l}\n"), true)
            }
        };
        self.clipboard = text.clone();
        self.clipboard_linewise = linewise;
        self.clipboard_export = Some(text);
        if cut {
            if linewise {
                self.delete_lines();
            } else {
                self.begin(GroupKind::Other);
                self.replace_selection("");
                self.end();
            }
        }
    }

    fn paste(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        let text = self.clipboard.clone();
        if self.clipboard_linewise && self.selection().is_none() {
            let at = Pos::new(self.cursor.line, 0);
            self.begin(GroupKind::Other);
            let end = self.apply(at, at, &text);
            self.cursor = Pos::new(end.line, self.cursor.col.min(self.buffer.line(end.line).len()));
            self.cursor = self.buffer.clamp(self.cursor);
            self.end();
        } else {
            self.insert_text(&text);
        }
    }

    /// Sets the internal clipboard from outside (e.g. a system clipboard
    /// integration).
    pub fn set_clipboard(&mut self, text: String) {
        self.clipboard = text;
        self.clipboard_linewise = false;
    }

    // ---- search ----------------------------------------------------------

    pub fn set_search(&mut self, query: &str) {
        self.search = if query.is_empty() { None } else { Some(query.to_string()) };
    }

    pub fn search_query(&self) -> Option<&str> {
        self.search.as_deref()
    }

    fn find_command(&mut self, forward: bool) -> Effect {
        if self.search.is_none() {
            return Effect::Prompt(PromptKind::Find);
        }
        self.find(forward);
        Effect::None
    }

    /// Finds the next/previous match of the current query, wrapping around.
    /// Smart case: case-insensitive unless the query contains upper case.
    pub fn find(&mut self, forward: bool) -> bool {
        self.end();
        let Some(query) = self.search.clone() else {
            return false;
        };
        let sensitive = query.chars().any(char::is_uppercase);
        let needle: Vec<char> = if sensitive {
            query.chars().collect()
        } else {
            query.chars().flat_map(char::to_lowercase).collect()
        };
        let n = self.buffer.line_count();
        let from = match (self.selection(), forward) {
            (Some((_, e)), true) => e,
            (Some((s, _)), false) => s,
            (None, _) => self.cursor,
        };
        for step in 0..=n {
            let l = if forward { (from.line + step) % n } else { (from.line + n - step % n) % n };
            let matches = line_matches(self.buffer.line(l), &needle, sensitive);
            let hit = if forward {
                matches
                    .into_iter()
                    .find(|(s, _)| step > 0 && step < n || (step == 0 && *s >= from.col) || (step == n && *s < from.col))
            } else {
                matches
                    .into_iter()
                    .rev()
                    .find(|(s, _)| step > 0 && step < n || (step == 0 && *s < from.col) || (step == n && *s >= from.col))
            };
            if let Some((s, e)) = hit {
                if step == n {
                    self.status = Some("Search wrapped".into());
                }
                self.anchor = Some(Pos::new(l, s));
                self.cursor = Pos::new(l, e);
                self.desired_col = None;
                return true;
            }
        }
        self.status = Some(format!("Not found: {query}"));
        false
    }

    pub fn goto_line(&mut self, line_1based: usize) {
        self.end();
        let l = line_1based.clamp(1, self.buffer.line_count()) - 1;
        self.cursor = Pos::new(l, 0);
        self.anchor = None;
        self.desired_col = None;
    }

    // ---- file ------------------------------------------------------------

    pub fn save(&mut self) -> io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "no file name"));
        };
        self.save_to(path)
    }

    pub fn save_to(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        self.end();
        let path = path.as_ref();
        fs::write(path, self.buffer.to_file_string())?;
        self.path = Some(path.to_path_buf());
        self.save_len = self.undo.len();
        Ok(())
    }

    fn save_reporting(&mut self) {
        match self.save() {
            Ok(()) => {
                let name = self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                self.status = Some(format!("Saved {name}"));
            }
            Err(e) => self.status = Some(format!("Save failed: {e}")),
        }
    }

    pub fn save_as_reporting(&mut self, path: &str) {
        match self.save_to(path) {
            Ok(()) => self.status = Some(format!("Saved {path}")),
            Err(e) => self.status = Some(format!("Save failed: {e}")),
        }
    }

    /// Number of grapheme clusters on the cursor line before the cursor
    /// (1-based column shown in the status bar).
    pub fn cursor_column(&self) -> usize {
        self.buffer.line(self.cursor.line)[..self.cursor.col].graphemes(true).count() + 1
    }
}

/// Byte ranges of all matches of `needle` in `line`.
fn line_matches(line: &str, needle: &[char], sensitive: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    for (i, _) in line.char_indices() {
        let mut it = line[i..].char_indices();
        let mut matched = 0;
        let mut end = i;
        let mut ok = true;
        'outer: while matched < needle.len() {
            let Some((off, ch)) = it.next() else {
                ok = false;
                break;
            };
            if sensitive {
                if ch != needle[matched] {
                    ok = false;
                    break;
                }
                matched += 1;
            } else {
                for lc in ch.to_lowercase() {
                    if matched >= needle.len() || lc != needle[matched] {
                        ok = false;
                        break 'outer;
                    }
                    matched += 1;
                }
            }
            end = i + off + ch.len_utf8();
        }
        if ok && matched == needle.len() {
            out.push((i, end));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Motion as M;

    fn ed(s: &str) -> Editor {
        let mut e = Editor::from_text(s);
        e.options.auto_indent = true;
        e
    }

    fn type_str(e: &mut Editor, s: &str) {
        for ch in s.chars() {
            if ch == '\n' {
                e.execute(Command::InsertNewline);
            } else {
                e.insert_text(&ch.to_string());
            }
        }
    }

    #[test]
    fn typing_and_undo_coalesce() {
        let mut e = ed("");
        type_str(&mut e, "hello");
        assert_eq!(e.text(), "hello");
        e.undo();
        assert_eq!(e.text(), "");
        e.redo();
        assert_eq!(e.text(), "hello");
    }

    #[test]
    fn shift_select_and_cut_paste() {
        let mut e = ed("hello world");
        e.execute(Command::Select(M::WordRight));
        assert_eq!(e.selected_text().as_deref(), Some("hello "));
        e.execute(Command::Cut);
        assert_eq!(e.text(), "world");
        e.execute(Command::Move(M::LineEnd));
        e.execute(Command::Paste);
        assert_eq!(e.text(), "worldhello ");
        e.execute(Command::Undo);
        assert_eq!(e.text(), "world");
    }

    #[test]
    fn left_collapses_selection() {
        let mut e = ed("abcdef");
        e.execute(Command::Move(M::Right));
        e.execute(Command::Select(M::Right));
        e.execute(Command::Select(M::Right));
        e.execute(Command::Move(M::Left));
        assert_eq!(e.cursor(), Pos::new(0, 1));
        assert!(e.selection().is_none());
    }

    #[test]
    fn auto_indent_and_backspace_join() {
        let mut e = ed("  foo");
        e.execute(Command::Move(M::DocEnd));
        e.execute(Command::InsertNewline);
        assert_eq!(e.text(), "  foo\n  ");
        e.execute(Command::Backspace);
        e.execute(Command::Backspace);
        e.execute(Command::Backspace);
        assert_eq!(e.text(), "  foo");
    }

    #[test]
    fn linewise_copy_paste_and_delete() {
        let mut e = ed("a\nb\nc");
        e.execute(Command::Move(M::Down));
        e.execute(Command::Copy);
        e.execute(Command::Move(M::DocStart));
        e.execute(Command::Paste);
        assert_eq!(e.text(), "b\na\nb\nc");
        // The cursor stays on the original line "a", which is deleted.
        e.execute(Command::DeleteLine);
        assert_eq!(e.text(), "b\nb\nc");
    }

    #[test]
    fn move_and_duplicate_lines() {
        let mut e = ed("a\nb\nc");
        e.execute(Command::MoveLineDown);
        assert_eq!(e.text(), "b\na\nc");
        assert_eq!(e.cursor().line, 1);
        e.execute(Command::MoveLineUp);
        assert_eq!(e.text(), "a\nb\nc");
        e.execute(Command::DuplicateLine);
        assert_eq!(e.text(), "a\na\nb\nc");
        e.execute(Command::Undo);
        assert_eq!(e.text(), "a\nb\nc");
    }

    #[test]
    fn indent_outdent_selection() {
        let mut e = ed("a\nb");
        e.execute(Command::SelectAll);
        e.execute(Command::InsertTab);
        assert_eq!(e.text(), "    a\n    b");
        e.execute(Command::Outdent);
        assert_eq!(e.text(), "a\nb");
    }

    #[test]
    fn find_wraps_and_smart_case() {
        let mut e = ed("foo Bar\nbar foo");
        e.set_search("bar");
        assert!(e.find(true));
        assert_eq!(e.selected_text().as_deref(), Some("Bar"));
        assert!(e.find(true));
        assert_eq!(e.cursor(), Pos::new(1, 3));
        assert!(e.find(true)); // wraps
        assert_eq!(e.selection().unwrap().0, Pos::new(0, 4));
        assert!(e.find(false));
        assert_eq!(e.selection().unwrap().0, Pos::new(1, 0));
        e.set_search("Bar");
        assert!(e.find(true));
        assert_eq!(e.selection().unwrap().0, Pos::new(0, 4));
    }

    #[test]
    fn vertical_movement_keeps_display_column() {
        let mut e = ed("あいう\nabcdef");
        e.execute(Command::Move(M::Right)); // after あ: display col 2
        e.execute(Command::Move(M::Down));
        assert_eq!(e.cursor(), Pos::new(1, 2));
    }

    #[test]
    fn modified_flag_tracks_save_point() {
        let mut e = ed("x");
        assert!(!e.is_modified());
        e.insert_text("y");
        assert!(e.is_modified());
        let dir = std::env::temp_dir().join(format!("kyoto-test-{}", std::process::id()));
        e.save_to(&dir).unwrap();
        assert!(!e.is_modified());
        e.undo();
        assert!(e.is_modified());
        let _ = fs::remove_file(dir);
    }
}
