//! Colours used by the renderer.
//!
//! Kept in one place so the look of the editor can be tweaked without touching
//! rendering logic. Syntax highlighting (added later) maps highlight names to
//! the [`Theme::syntax`] colours here.

use crossterm::style::Color;

/// A colour scheme. The defaults are a muted dark theme that reads well on a
/// typical terminal.
pub struct Theme {
    /// Dimmed colour for the line-number gutter.
    pub gutter: Color,
    /// Brighter gutter colour for the cursor's line.
    pub gutter_current: Color,
    pub status_fg: Color,
    pub status_bg: Color,
    /// Background of the visual-mode selection.
    pub selection_bg: Color,
    /// The `~` markers shown past the end of the buffer.
    pub end_of_buffer: Color,
}

impl Default for Theme {
    fn default() -> Theme {
        Theme {
            gutter: Color::DarkGrey,
            gutter_current: Color::Grey,
            status_fg: Color::Black,
            status_bg: Color::Cyan,
            selection_bg: Color::DarkBlue,
            end_of_buffer: Color::DarkGrey,
        }
    }
}

/// Map a tree-sitter highlight name (such as `keyword` or `string`) to a
/// colour. Returns `None` for names lux doesn't theme, which render as default
/// foreground. Used by the syntax module.
pub fn highlight_color(name: &str) -> Option<Color> {
    // Match on the first component so `function.method` falls back to
    // `function`, etc.
    let base = name.split('.').next().unwrap_or(name);
    let color = match base {
        "keyword" => Color::Magenta,
        "function" => Color::Blue,
        "type" => Color::Yellow,
        "string" => Color::Green,
        "comment" => Color::DarkGrey,
        "constant" => Color::Cyan,
        "number" => Color::Cyan,
        "operator" => Color::Grey,
        "property" => Color::Red,
        "variable" => return None,
        "punctuation" => Color::Grey,
        _ => return None,
    };
    Some(color)
}
