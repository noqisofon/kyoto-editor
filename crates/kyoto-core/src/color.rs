//! Backend-independent colors used by themes.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// The terminal's default color.
    Reset,
    Rgb(u8, u8, u8),
    /// An entry of the 256 color palette (0-15 are the named ANSI colors).
    Indexed(u8),
}

const NAMED: &[(&str, u8)] = &[
    ("black", 0),
    ("red", 1),
    ("green", 2),
    ("yellow", 3),
    ("blue", 4),
    ("magenta", 5),
    ("cyan", 6),
    ("gray", 7),
    ("grey", 7),
    ("dark_gray", 8),
    ("dark_grey", 8),
    ("light_red", 9),
    ("light_green", 10),
    ("light_yellow", 11),
    ("light_blue", 12),
    ("light_magenta", 13),
    ("light_cyan", 14),
    ("white", 15),
];

impl Color {
    /// Accepts `#rrggbb`, `#rgb`, a color name (`blue`, `light_cyan`, ...),
    /// `reset`/`default`, or `ansi:N` for a palette index.
    pub fn parse(s: &str) -> Result<Color, String> {
        let t = s.trim().to_ascii_lowercase();
        if t == "reset" || t == "default" || t == "none" {
            return Ok(Color::Reset);
        }
        if let Some(hex) = t.strip_prefix('#') {
            let v = |i: usize, w: usize| u8::from_str_radix(&hex[i..i + w], 16);
            let bad = || format!("invalid color `{s}`");
            if !hex.is_ascii() {
                return Err(bad());
            }
            return match hex.len() {
                6 => Ok(Color::Rgb(
                    v(0, 2).map_err(|_| bad())?,
                    v(2, 2).map_err(|_| bad())?,
                    v(4, 2).map_err(|_| bad())?,
                )),
                3 => {
                    let d = |i: usize| v(i, 1).map(|x| x * 17).map_err(|_| bad());
                    Ok(Color::Rgb(d(0)?, d(1)?, d(2)?))
                }
                _ => Err(bad()),
            };
        }
        if let Some(n) = t.strip_prefix("ansi:") {
            return n.parse::<u8>().map(Color::Indexed).map_err(|_| format!("invalid color `{s}`"));
        }
        NAMED
            .iter()
            .find(|(n, _)| *n == t)
            .map(|(_, i)| Color::Indexed(*i))
            .ok_or_else(|| format!("unknown color `{s}`"))
    }
}

/// Colors for the editor chrome. `None` means "use the terminal default".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    pub selection_fg: Option<Color>,
    pub selection_bg: Color,
    pub current_line_bg: Option<Color>,
    pub line_number: Color,
    pub current_line_number: Color,
    pub status_fg: Color,
    pub status_bg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            foreground: None,
            background: None,
            selection_fg: None,
            selection_bg: Color::Indexed(4),
            current_line_bg: None,
            line_number: Color::Indexed(8),
            current_line_number: Color::Indexed(7),
            status_fg: Color::Indexed(0),
            status_bg: Color::Indexed(7),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_colors() {
        assert_eq!(Color::parse("#ff8000"), Ok(Color::Rgb(255, 128, 0)));
        assert_eq!(Color::parse("#f80"), Ok(Color::Rgb(255, 136, 0)));
        assert_eq!(Color::parse("Blue"), Ok(Color::Indexed(4)));
        assert_eq!(Color::parse("ansi:200"), Ok(Color::Indexed(200)));
        assert!(Color::parse("#12").is_err());
        assert!(Color::parse("#ééé").is_err());
        assert!(Color::parse("nope").is_err());
    }
}
