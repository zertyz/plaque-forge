//! Normalization of `cv::waitKey` codes across highgui backends.
//!
//! The Qt backend reports extended Qt keycodes while other backends report
//! compact ASCII-like codes; both are mapped onto one enum so behavior never
//! depends on the installed toolkit.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Unknown,
}

const QT_BASE: i32 = 1 << 24; // 0x01000000

/// Map a raw `waitKey` return value.
pub fn normalize(code: i32) -> Key {
    match code {
        27 => Key::Esc,
        10 | 13 => Key::Enter,
        8 | 127 => Key::Backspace,
        // Compact (GTK-like) special codes used by several backends.
        80 => Key::Home,
        81 => Key::Left,
        82 => Key::Up,
        83 => Key::Right,
        84 => Key::Down,
        85 => Key::PageUp,
        86 => Key::PageDown,
        87 => Key::End,
        _ if (0x20..0x7f).contains(&code) => Key::Char(code as u8 as char),
        // Qt extended codes.
        c if c >= QT_BASE => qt_key(c - QT_BASE),
        _ => Key::Unknown,
    }
}

fn qt_key(rest: i32) -> Key {
    match rest {
        0x01000000 => Key::Esc,       // Qt::Key_Escape
        0x01000004 => Key::Enter,     // Qt::Key_Return
        0x01000005 => Key::Enter,     // Qt::Key_Enter (numpad)
        0x01000007 => Key::Backspace, // Qt::Key_Backspace
        0x01000010 => Key::Home,
        0x01000011 => Key::End,
        0x01000012 => Key::Left,
        0x01000013 => Key::Up,
        0x01000014 => Key::Right,
        0x01000015 => Key::Down,
        0x01000016 => Key::PageUp,
        0x01000017 => Key::PageDown,
        0x0100007f => Key::Delete,
        c if (0x20..0x7f).contains(&c) => Key::Char(c as u8 as char),
        _ => Key::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_control_keys_normalize() {
        assert_eq!(normalize(27), Key::Esc);
        assert_eq!(normalize(13), Key::Enter);
        assert_eq!(normalize(10), Key::Enter);
        assert_eq!(normalize(47), Key::Char('/'));
        assert_eq!(normalize(b'a' as i32), Key::Char('a'));
        assert_eq!(normalize(8), Key::Backspace);
        assert_eq!(normalize(-1), Key::Unknown);
    }

    #[test]
    fn compact_special_codes_match_the_documented_table() {
        assert_eq!(normalize(80), Key::Home);
        assert_eq!(normalize(81), Key::Left);
        assert_eq!(normalize(82), Key::Up);
        assert_eq!(normalize(83), Key::Right);
        assert_eq!(normalize(84), Key::Down);
        assert_eq!(normalize(85), Key::PageUp);
        assert_eq!(normalize(86), Key::PageDown);
        assert_eq!(normalize(87), Key::End);
    }

    #[test]
    fn qt_extended_codes_normalize() {
        let k = |rest| normalize(QT_BASE + rest);
        assert_eq!(k(0x01000000), Key::Esc);
        assert_eq!(k(0x01000013), Key::Up);
        assert_eq!(k(0x01000015), Key::Down);
        assert_eq!(k(0x01000012), Key::Left);
        assert_eq!(k(0x01000014), Key::Right);
        assert_eq!(k(0x01000016), Key::PageUp);
        assert_eq!(k(0x01000017), Key::PageDown);
        assert_eq!(k(0x01000010), Key::Home);
        assert_eq!(k(0x01000005), Key::Enter);
        assert_eq!(k(0x0100007f), Key::Delete);
        assert_eq!(k(b'/' as i32), Key::Char('/'));
    }
}
