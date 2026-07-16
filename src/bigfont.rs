//! A tiny 5-row block font for the big current-value display.
//!
//! Renders digits, `.`, `-`, and space as 5 lines of block characters so the
//! current glucose reads across a room.

/// Number of rows each glyph occupies.
pub const ROWS: usize = 5;

/// 5×3 block glyphs for the characters we need.
fn glyph(c: char) -> [&'static str; ROWS] {
    match c {
        '0' => ["███", "█ █", "█ █", "█ █", "███"],
        '1' => ["  █", "  █", "  █", "  █", "  █"],
        '2' => ["███", "  █", "███", "█  ", "███"],
        '3' => ["███", "  █", "███", "  █", "███"],
        '4' => ["█ █", "█ █", "███", "  █", "  █"],
        '5' => ["███", "█  ", "███", "  █", "███"],
        '6' => ["███", "█  ", "███", "█ █", "███"],
        '7' => ["███", "  █", "  █", "  █", "  █"],
        '8' => ["███", "█ █", "███", "█ █", "███"],
        '9' => ["███", "█ █", "███", "  █", "███"],
        '.' => ["   ", "   ", "   ", "   ", " █ "],
        '-' => ["   ", "   ", "███", "   ", "   "],
        _ => ["   ", "   ", "   ", "   ", "   "], // space / unknown
    }
}

/// Render `text` as `ROWS` lines of block characters, glyphs separated by a
/// space column.
pub fn render(text: &str) -> Vec<String> {
    let mut lines = vec![String::new(); ROWS];
    for (i, c) in text.chars().enumerate() {
        let g = glyph(c);
        for (row, cell) in g.iter().enumerate() {
            if i > 0 {
                lines[row].push(' ');
            }
            lines[row].push_str(cell);
        }
    }
    lines
}

/// The rendered width in columns for `text` (for layout decisions).
pub fn width(text: &str) -> u16 {
    let n = text.chars().count();
    if n == 0 {
        0
    } else {
        (n * 3 + (n - 1)) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_five_rows() {
        let out = render("8.6");
        assert_eq!(out.len(), ROWS);
        // "8" + sep + "." + sep + "6" = 3+1+3+1+3 = 11 columns.
        assert_eq!(out[0].chars().count(), 11);
        assert_eq!(width("8.6"), 11);
    }

    #[test]
    fn empty_is_zero_width() {
        assert_eq!(width(""), 0);
    }
}
