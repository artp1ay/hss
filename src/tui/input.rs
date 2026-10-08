//! Shared single-line text editing: cursor movement, insert/delete at cursor.
//! Cursor is a char index (not a byte offset), so UTF-8 input is safe.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    style::{Modifier, Style},
    text::Span,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

fn byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// Apply an editing key to `(buf, cursor)`. Returns true when the key was consumed.
pub fn handle(buf: &mut String, cursor: &mut usize, key: KeyEvent) -> bool {
    let len = buf.chars().count();
    if *cursor > len {
        *cursor = len;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Left => {
            *cursor = cursor.saturating_sub(1);
            true
        }
        KeyCode::Right => {
            if *cursor < len {
                *cursor += 1;
            }
            true
        }
        KeyCode::Home => {
            *cursor = 0;
            true
        }
        KeyCode::End => {
            *cursor = len;
            true
        }
        KeyCode::Backspace => {
            if *cursor > 0 {
                let b = byte_idx(buf, *cursor - 1);
                buf.remove(b);
                *cursor -= 1;
            }
            true
        }
        KeyCode::Delete => {
            if *cursor < len {
                let b = byte_idx(buf, *cursor);
                buf.remove(b);
            }
            true
        }
        KeyCode::Char('a') if ctrl => {
            *cursor = 0;
            true
        }
        KeyCode::Char('e') if ctrl => {
            *cursor = len;
            true
        }
        KeyCode::Char('u') if ctrl => {
            let b = byte_idx(buf, *cursor);
            buf.replace_range(..b, "");
            *cursor = 0;
            true
        }
        KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
            let b = byte_idx(buf, *cursor);
            buf.insert(b, c);
            *cursor += 1;
            true
        }
        _ => false,
    }
}

/// Render `value` with a block cursor at `cursor` (reversed cell; trailing space when at end).
pub fn spans(value: &str, cursor: usize, style: Style) -> Vec<Span<'static>> {
    let chars: Vec<char> = value.chars().collect();
    let c = cursor.min(chars.len());
    let before: String = chars[..c].iter().collect();
    let at: String = chars
        .get(c)
        .map(|ch| ch.to_string())
        .unwrap_or_else(|| " ".into());
    let after: String = chars
        .get(c + 1..)
        .map(|s| s.iter().collect())
        .unwrap_or_default();
    vec![
        Span::styled(before, style),
        Span::styled(at, style.add_modifier(Modifier::REVERSED)),
        Span::styled(after, style),
    ]
}

/// Visual terminal width of a string.
pub fn str_width(s: &str) -> usize {
    s.width()
}

/// Visual terminal width up to character index `char_idx`.
pub fn char_idx_to_display_col(s: &str, char_idx: usize) -> usize {
    s.chars()
        .take(char_idx)
        .map(|c| c.width().unwrap_or(0))
        .sum()
}

/// Char length helper for placing the cursor at the end of a field.
pub fn end_of(s: &str) -> usize {
    s.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn insert_move_and_delete_at_cursor() {
        let mut buf = String::new();
        let mut cur = 0;
        for c in "abc".chars() {
            assert!(handle(&mut buf, &mut cur, k(KeyCode::Char(c))));
        }
        assert_eq!((buf.as_str(), cur), ("abc", 3));

        handle(&mut buf, &mut cur, k(KeyCode::Left));
        handle(&mut buf, &mut cur, k(KeyCode::Left));
        assert_eq!(cur, 1);

        handle(&mut buf, &mut cur, k(KeyCode::Char('X')));
        assert_eq!((buf.as_str(), cur), ("aXbc", 2));

        handle(&mut buf, &mut cur, k(KeyCode::Backspace));
        assert_eq!((buf.as_str(), cur), ("abc", 1));

        handle(&mut buf, &mut cur, k(KeyCode::Delete));
        assert_eq!((buf.as_str(), cur), ("ac", 1));

        handle(&mut buf, &mut cur, k(KeyCode::Home));
        assert_eq!(cur, 0);
        handle(&mut buf, &mut cur, k(KeyCode::End));
        assert_eq!(cur, 2);
    }

    #[test]
    fn utf8_safe_and_bounds() {
        let mut buf = "привет".to_string();
        let mut cur = 3;
        handle(&mut buf, &mut cur, k(KeyCode::Backspace));
        assert_eq!((buf.as_str(), cur), ("првет", 2));

        let mut cur = 0;
        handle(&mut buf, &mut cur, k(KeyCode::Left)); // no underflow
        assert_eq!(cur, 0);
        let mut cur = 99; // clamped
        handle(&mut buf, &mut cur, k(KeyCode::Right));
        assert_eq!(cur, buf.chars().count());
    }

    #[test]
    fn non_editing_keys_not_consumed() {
        let mut buf = "x".to_string();
        let mut cur = 1;
        assert!(!handle(&mut buf, &mut cur, k(KeyCode::Tab)));
        assert!(!handle(&mut buf, &mut cur, k(KeyCode::Enter)));
        assert!(!handle(&mut buf, &mut cur, k(KeyCode::Up)));
    }

    #[test]
    fn unicode_display_widths() {
        assert_eq!(str_width("hello"), 5);
        // "你好" has 2 chars, each width 2 -> total width 4
        assert_eq!(str_width("你好"), 4);
        assert_eq!(char_idx_to_display_col("你好world", 2), 4);
        assert_eq!(char_idx_to_display_col("你好world", 4), 6);
    }
}
