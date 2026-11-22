//! Line based text buffer.
//!
//! Positions are `(line, byte column)` pairs. Columns always sit on grapheme
//! cluster boundaries, which the movement helpers in this module guarantee.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// A position in the buffer: line index and *byte* offset within the line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

/// Line ending style detected on load and restored on save.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Buffer {
    lines: Vec<String>,
    pub line_ending: LineEnding,
    pub final_newline: bool,
}

impl Default for Buffer {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
            line_ending: LineEnding::Lf,
            final_newline: true,
        }
    }
}

impl Buffer {
    pub fn from_text(text: &str) -> Self {
        let line_ending = if text.contains("\r\n") {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        };
        let final_newline = text.is_empty() || text.ends_with('\n');
        let body = text.strip_suffix('\n').unwrap_or(text);
        let mut lines: Vec<String> = body
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
            .collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        Self {
            lines,
            line_ending,
            final_newline,
        }
    }

    /// Serialises the buffer using its original line ending style.
    pub fn to_file_string(&self) -> String {
        let eol = self.line_ending.as_str();
        let mut out = self.lines.join(eol);
        if self.final_newline {
            out.push_str(eol);
        }
        out
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line(&self, i: usize) -> &str {
        &self.lines[i]
    }

    pub fn end_pos(&self) -> Pos {
        let line = self.lines.len() - 1;
        Pos::new(line, self.lines[line].len())
    }

    /// Text between two positions (`start <= end`), lines joined by `\n`.
    pub fn slice(&self, start: Pos, end: Pos) -> String {
        if start.line == end.line {
            return self.lines[start.line][start.col..end.col].to_string();
        }
        let mut out = String::new();
        out.push_str(&self.lines[start.line][start.col..]);
        for l in &self.lines[start.line + 1..end.line] {
            out.push('\n');
            out.push_str(l);
        }
        out.push('\n');
        out.push_str(&self.lines[end.line][..end.col]);
        out
    }

    /// Replaces `start..end` with `text` (which must use `\n` only) and
    /// returns the position just after the inserted text.
    pub fn replace(&mut self, start: Pos, end: Pos, text: &str) -> Pos {
        debug_assert!(start <= end);
        let head = &self.lines[start.line][..start.col];
        let tail = &self.lines[end.line][end.col..];
        let mut parts = text.split('\n');
        let first = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        let (new_lines, end_pos) = if rest.is_empty() {
            (
                vec![format!("{head}{first}{tail}")],
                Pos::new(start.line, start.col + first.len()),
            )
        } else {
            let mut v = Vec::with_capacity(rest.len() + 1);
            v.push(format!("{head}{first}"));
            for r in &rest[..rest.len() - 1] {
                v.push((*r).to_string());
            }
            let last = rest[rest.len() - 1];
            v.push(format!("{last}{tail}"));
            (v, Pos::new(start.line + rest.len(), last.len()))
        };
        self.lines.splice(start.line..=end.line, new_lines);
        end_pos
    }

    /// Clamps a position into the buffer and onto a grapheme boundary.
    pub fn clamp(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines.len() - 1);
        let text = &self.lines[line];
        let mut col = p.col.min(text.len());
        while !text.is_char_boundary(col) {
            col -= 1;
        }
        // Snap back to a grapheme boundary.
        let mut last = 0;
        for (i, _) in text.grapheme_indices(true) {
            if i > col {
                break;
            }
            last = i;
        }
        if col != text.len() {
            col = last;
        }
        Pos::new(line, col)
    }
}

/// Position after inserting `text` at `start`.
pub fn advance(start: Pos, text: &str) -> Pos {
    match text.rfind('\n') {
        None => Pos::new(start.line, start.col + text.len()),
        Some(i) => Pos::new(
            start.line + text.matches('\n').count(),
            text.len() - i - 1,
        ),
    }
}

pub fn next_boundary(line: &str, col: usize) -> usize {
    line[col..]
        .graphemes(true)
        .next()
        .map_or(col, |g| col + g.len())
}

pub fn prev_boundary(line: &str, col: usize) -> usize {
    line[..col]
        .graphemes(true)
        .next_back()
        .map_or(col, |g| col - g.len())
}

/// Display column (terminal cells) of byte column `col`.
pub fn display_col(line: &str, col: usize, tab_width: usize) -> usize {
    let mut x = 0;
    for g in line[..col].graphemes(true) {
        x += grapheme_width(g, x, tab_width);
    }
    x
}

/// Byte column whose display column is closest to (not past) `target`.
pub fn col_from_display(line: &str, target: usize, tab_width: usize) -> usize {
    let mut x = 0;
    for (i, g) in line.grapheme_indices(true) {
        let w = grapheme_width(g, x, tab_width);
        if x + w > target {
            // Round to the nearer edge for wide glyphs.
            return if target - x > w / 2 && w > 1 { i + g.len() } else { i };
        }
        x += w;
    }
    line.len()
}

/// Width of one grapheme at display column `x` (tabs expand to the next stop).
pub fn grapheme_width(g: &str, x: usize, tab_width: usize) -> usize {
    if g == "\t" {
        let tw = tab_width.max(1);
        tw - x % tw
    } else if g.chars().next().is_some_and(char::is_control) {
        // Control characters are drawn as a one cell placeholder.
        1
    } else {
        UnicodeWidthStr::width(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_replace() {
        let mut b = Buffer::from_text("abc\ndef\n");
        assert_eq!(b.line_count(), 2);
        let end = b.replace(Pos::new(0, 1), Pos::new(1, 1), "X\nY\nZ");
        assert_eq!(end, Pos::new(2, 1));
        assert_eq!(b.to_file_string(), "aX\nY\nZef\n");
        assert_eq!(b.slice(Pos::new(0, 1), end), "X\nY\nZ");
    }

    #[test]
    fn crlf_preserved() {
        let b = Buffer::from_text("a\r\nb");
        assert_eq!(b.line_ending, LineEnding::CrLf);
        assert!(!b.final_newline);
        assert_eq!(b.to_file_string(), "a\r\nb");
    }

    #[test]
    fn graphemes_and_width() {
        let s = "あ\u{3099}い"; // combining dakuten
        assert_eq!(next_boundary(s, 0), "あ\u{3099}".len());
        assert_eq!(display_col("a\tb", 2, 4), 4);
        assert_eq!(display_col("あい", 3, 4), 2);
        assert_eq!(col_from_display("あい", 2, 4), 3);
    }
}
