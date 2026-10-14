//! Semantic ANSI colors; ordinary text follows the terminal's foreground.
use ratatui::style::Color;

pub const TEXT: Color = Color::Reset;
pub const MUTED: Color = Color::Gray;
pub const FOCUS: Color = Color::Cyan;
pub const SELECTION: Color = Color::DarkGray;
pub const SUCCESS: Color = Color::Green;
pub const INFO: Color = Color::Cyan;
pub const WARNING: Color = Color::Yellow;
pub const ERROR: Color = Color::Red;
