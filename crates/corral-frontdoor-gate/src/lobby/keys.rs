//! The keys in what a client's terminal sends, which arrives raw: the gate holds no pty of its
//! own for the lobby.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Char(char),
    Enter,
    Backspace,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Tab,
    /// Ctrl-C.
    Interrupt,
    /// Ctrl-D.
    End,
}

/// The keys in one read from the terminal. An escape that ends the read, or that another escape
/// follows, is the Esc key: a terminal sends each sequence whole.
pub(crate) fn keys(data: &[u8]) -> Vec<Key> {
    let text = String::from_utf8_lossy(data);
    let mut chars = text.chars().peekable();
    let mut keys = Vec::new();
    while let Some(c) = chars.next() {
        let key = match c {
            '\x1b' => match chars.peek() {
                Some('[' | 'O') => {
                    chars.next();
                    let mut last = None;
                    for c in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            last = Some(c);
                            break;
                        }
                    }
                    match last {
                        Some('A') => Key::Up,
                        Some('B') => Key::Down,
                        Some('C') => Key::Right,
                        Some('D') => Key::Left,
                        _ => continue,
                    }
                }
                _ => Key::Esc,
            },
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                Key::Enter
            }
            '\n' => Key::Enter,
            '\x7f' | '\x08' => Key::Backspace,
            '\t' => Key::Tab,
            '\x03' => Key::Interrupt,
            '\x04' => Key::End,
            c if c.is_control() => continue,
            c => Key::Char(c),
        };
        keys.push(key);
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::{Key, keys};

    #[test]
    fn a_read_decodes_into_its_keys() {
        assert_eq!(
            keys(b"a\xc3\xa9 \r\x7f\x1b[A\x1bOB\x1b[C\x1b[D\t\x03\x04"),
            [
                Key::Char('a'),
                Key::Char('é'),
                Key::Char(' '),
                Key::Enter,
                Key::Backspace,
                Key::Up,
                Key::Down,
                Key::Right,
                Key::Left,
                Key::Tab,
                Key::Interrupt,
                Key::End,
            ]
        );
    }

    #[test]
    fn an_escape_alone_is_the_esc_key_and_other_sequences_are_skipped() {
        assert_eq!(keys(b"\x1b"), [Key::Esc]);
        assert_eq!(keys(b"\x1b\x1b"), [Key::Esc, Key::Esc]);
        assert_eq!(keys(b"\x1b[3~x"), [Key::Char('x')]);
        assert_eq!(keys(b"\x1b[1;5Ax"), [Key::Up, Key::Char('x')]);
        assert_eq!(keys(b"\r\n"), [Key::Enter]);
    }
}
