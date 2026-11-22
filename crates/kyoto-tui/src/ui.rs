//! Rendering with ratatui.

use kyoto_core::buffer::{display_col, grapheme_width};
use kyoto_core::{Color as CColor, Pos};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::App;

fn conv(c: CColor) -> Color {
    match c {
        CColor::Reset => Color::Reset,
        CColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        CColor::Indexed(i) => Color::Indexed(i),
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let [text_area, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(f.area());

    let theme = app.config.theme.clone();
    let mut base = Style::default();
    if let Some(c) = theme.foreground {
        base = base.fg(conv(c));
    }
    if let Some(c) = theme.background {
        base = base.bg(conv(c));
    }
    f.render_widget(Block::default().style(base), f.area());

    app.editor.set_page_size(text_area.height as usize);
    let line_count = app.editor.buffer().line_count();
    let gutter = if app.config.line_numbers { line_count.to_string().len().max(3) + 1 } else { 0 };
    let width = (text_area.width as usize).saturating_sub(gutter).max(1);
    let height = text_area.height as usize;

    // Scroll so the cursor stays visible.
    let cursor = app.editor.cursor();
    let tab = app.config.edit.tab_width;
    let cursor_x = display_col(app.editor.buffer().line(cursor.line), cursor.col, tab);
    if cursor.line < app.top {
        app.top = cursor.line;
    } else if cursor.line >= app.top + height {
        app.top = cursor.line + 1 - height;
    }
    if cursor_x < app.left {
        app.left = cursor_x;
    } else if cursor_x >= app.left + width {
        app.left = cursor_x + 1 - width;
    }

    let selection = app.editor.selection();
    let mut lines = Vec::with_capacity(height);
    for row in 0..height {
        let l = app.top + row;
        if l >= line_count {
            lines.push(Line::default());
            continue;
        }
        let is_cur = l == cursor.line;
        let mut spans = Vec::new();
        if gutter > 0 {
            let color = if is_cur { theme.current_line_number } else { theme.line_number };
            spans.push(Span::styled(
                format!("{:>w$} ", l + 1, w = gutter - 1),
                base.fg(conv(color)),
            ));
        }
        spans.extend(render_line(app, l, is_cur, selection, width, base));
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), text_area);

    // Status / prompt line.
    let status_style = Style::default().fg(conv(theme.status_fg)).bg(conv(theme.status_bg));
    let total = status_area.width as usize;
    let (status, prompt_cursor) = match &app.prompt {
        Some(p) => {
            let head = format!(" {}: ", p.label());
            let x = head.width() + p.input.as_str().width();
            (format!("{head}{}", p.input), Some(x))
        }
        None => {
            let name = app
                .editor
                .path()
                .map(|p| p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "[No Name]".into());
            let dirty = if app.editor.is_modified() { " [+]" } else { "" };
            let left = match &app.message {
                Some(m) => format!(" {name}{dirty}  |  {m}"),
                None => format!(" {name}{dirty}"),
            };
            let right = format!("Ln {}, Col {}  ", cursor.line + 1, app.editor.cursor_column());
            let pad = total.saturating_sub(left.as_str().width() + right.width());
            (format!("{left}{}{right}", " ".repeat(pad)), None)
        }
    };
    f.render_widget(Paragraph::new(status).style(status_style), status_area);

    match prompt_cursor {
        Some(x) => f.set_cursor_position((status_area.x + (x as u16).min(status_area.width.saturating_sub(1)), status_area.y)),
        None => f.set_cursor_position((
            text_area.x + (gutter + cursor_x - app.left) as u16,
            text_area.y + (cursor.line - app.top) as u16,
        )),
    }
}

/// Renders one buffer line into styled spans covering `width` cells starting
/// at display column `app.left`.
fn render_line(
    app: &App,
    l: usize,
    is_cur: bool,
    selection: Option<(Pos, Pos)>,
    width: usize,
    base: Style,
) -> Vec<Span<'static>> {
    use unicode_segmentation::UnicodeSegmentation;

    let theme = &app.config.theme;
    let tab = app.config.edit.tab_width;
    let text = app.editor.buffer().line(l);
    let mut line_style = base;
    if is_cur {
        if let Some(bg) = theme.current_line_bg {
            line_style = line_style.bg(conv(bg));
        }
    }
    let mut sel_style = line_style.bg(conv(theme.selection_bg));
    if let Some(fg) = theme.selection_fg {
        sel_style = sel_style.fg(conv(fg));
    }

    // Selected byte range on this line, plus whether it continues past EOL.
    let (sel_lo, sel_hi, past_eol) = match selection {
        Some((s, e)) if (s.line..=e.line).contains(&l) => (
            if l == s.line { s.col } else { 0 },
            if l == e.line { e.col } else { text.len() },
            l < e.line,
        ),
        _ => (0, 0, false),
    };

    let left = app.left;
    let right = left + width;
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_style = line_style;
    let push = |s: &str, style: Style, spans: &mut Vec<Span<'static>>| {
        if s.is_empty() {
            return;
        }
        match spans.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push_str(s),
            _ => spans.push(Span::styled(s.to_string(), style)),
        }
    };

    let mut x = 0;
    for (i, g) in text.grapheme_indices(true) {
        if x >= right {
            break;
        }
        let w = grapheme_width(g, x, tab);
        let style = if i >= sel_lo && i < sel_hi { sel_style } else { line_style };
        let (gx0, gx1) = (x, x + w);
        x = gx1;
        if gx1 <= left {
            continue;
        }
        // Fully visible plain glyph.
        let shown: String = if g == "\t" || gx0 < left || gx1 > right {
            " ".repeat(gx1.min(right) - gx0.max(left))
        } else if g.chars().next().is_some_and(char::is_control) {
            "·".to_string()
        } else {
            g.to_string()
        };
        if style != run_style {
            push(&run, run_style, &mut spans);
            run.clear();
            run_style = style;
        }
        run.push_str(&shown);
    }
    push(&run, run_style, &mut spans);
    if past_eol && x < right {
        spans.push(Span::styled(" ", sel_style));
    }
    spans
}
