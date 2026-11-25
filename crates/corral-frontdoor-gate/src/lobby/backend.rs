//! A ratatui backend for a terminal at the other end of an SSH channel: it writes the escape
//! sequences a draw makes into a buffer the lobby sends on the channel, and takes the terminal's
//! size from the client's pty request and window changes.

use std::convert::Infallible;
use std::fmt::Write as _;

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Modifier};

pub(crate) struct ChannelBackend {
    out: String,
    size: Size,
    cursor: Position,
    /// Whether colors are drawn, or only bold, reversed and the like.
    color: bool,
    drawn: Option<(Color, Color, Modifier)>,
}

impl ChannelBackend {
    pub(crate) fn new(size: Size, color: bool) -> Self {
        Self {
            out: String::new(),
            size,
            cursor: Position::ORIGIN,
            color,
            drawn: None,
        }
    }

    pub(crate) fn resize(&mut self, size: Size) {
        self.size = size;
    }

    /// What the draws since the last call wrote.
    pub(crate) fn take(&mut self) -> String {
        std::mem::take(&mut self.out)
    }

    pub(crate) fn write(&mut self, text: &str) {
        self.out.push_str(text);
    }

    fn style(&mut self, fg: Color, bg: Color, modifier: Modifier) {
        let (fg, bg) = if self.color {
            (fg, bg)
        } else {
            (Color::Reset, Color::Reset)
        };
        if self.drawn == Some((fg, bg, modifier)) {
            return;
        }
        self.drawn = Some((fg, bg, modifier));
        self.out.push_str("\x1b[0");
        for (flag, code) in [
            (Modifier::BOLD, "1"),
            (Modifier::DIM, "2"),
            (Modifier::ITALIC, "3"),
            (Modifier::UNDERLINED, "4"),
            (Modifier::SLOW_BLINK, "5"),
            (Modifier::RAPID_BLINK, "6"),
            (Modifier::REVERSED, "7"),
            (Modifier::HIDDEN, "8"),
            (Modifier::CROSSED_OUT, "9"),
        ] {
            if modifier.contains(flag) {
                let _ = write!(self.out, ";{code}");
            }
        }
        if let Some(code) = sgr(fg, 30) {
            let _ = write!(self.out, ";{code}");
        }
        if let Some(code) = sgr(bg, 40) {
            let _ = write!(self.out, ";{code}");
        }
        self.out.push('m');
    }

    fn move_to(&mut self, position: Position) {
        let _ = write!(self.out, "\x1b[{};{}H", position.y + 1, position.x + 1);
        self.cursor = position;
    }
}

/// The SGR parameters of a color, from `base` (30 for the foreground, 40 for the background).
fn sgr(color: Color, base: u8) -> Option<String> {
    let basic = |n: u8| Some((base + n).to_string());
    let bright = |n: u8| Some((base + 60 + n).to_string());
    match color {
        Color::Reset => None,
        Color::Black => basic(0),
        Color::Red => basic(1),
        Color::Green => basic(2),
        Color::Yellow => basic(3),
        Color::Blue => basic(4),
        Color::Magenta => basic(5),
        Color::Cyan => basic(6),
        Color::Gray => basic(7),
        Color::DarkGray => bright(0),
        Color::LightRed => bright(1),
        Color::LightGreen => bright(2),
        Color::LightYellow => bright(3),
        Color::LightBlue => bright(4),
        Color::LightMagenta => bright(5),
        Color::LightCyan => bright(6),
        Color::White => bright(7),
        Color::Indexed(n) => Some(format!("{};5;{n}", base + 8)),
        Color::Rgb(r, g, b) => Some(format!("{};2;{r};{g};{b}", base + 8)),
    }
}

impl Backend for ChannelBackend {
    type Error = Infallible;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Infallible>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let mut next: Option<Position> = None;
        for (x, y, cell) in content {
            let at = Position { x, y };
            if next != Some(at) {
                self.move_to(at);
            }
            self.style(cell.fg, cell.bg, cell.modifier);
            self.out.push_str(cell.symbol());
            next = Some(Position { x: x + 1, y });
        }
        if next.is_some() {
            self.out.push_str("\x1b[0m");
            self.drawn = None;
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Infallible> {
        self.out.push_str("\x1b[?25l");
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), Infallible> {
        self.out.push_str("\x1b[?25h");
        Ok(())
    }

    fn get_cursor_position(&mut self) -> Result<Position, Infallible> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Infallible> {
        self.move_to(position.into());
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Infallible> {
        self.clear_region(ClearType::All)
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Infallible> {
        self.out.push_str(match clear_type {
            ClearType::All => "\x1b[2J",
            ClearType::AfterCursor => "\x1b[0J",
            ClearType::BeforeCursor => "\x1b[1J",
            ClearType::CurrentLine => "\x1b[2K",
            ClearType::UntilNewLine => "\x1b[0K",
        });
        Ok(())
    }

    fn size(&self) -> Result<Size, Infallible> {
        Ok(self.size)
    }

    fn window_size(&mut self) -> Result<WindowSize, Infallible> {
        Ok(WindowSize {
            columns_rows: self.size,
            pixels: Size::ZERO,
        })
    }

    fn flush(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::layout::Size;
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};

    use super::ChannelBackend;

    fn drawn(color: bool) -> String {
        let mut terminal = Terminal::new(ChannelBackend::new(Size::new(20, 2), color)).unwrap();
        terminal
            .draw(|frame| {
                let line = Line::from(vec![
                    Span::raw("a "),
                    Span::styled("red", Style::new().fg(Color::Red)),
                ]);
                frame.render_widget(line, frame.area());
            })
            .unwrap();
        terminal.backend_mut().take()
    }

    #[test]
    fn a_draw_writes_its_cells_with_their_colors_only_when_colors_are_on() {
        let colored = drawn(true);
        assert!(
            colored.contains("\x1b[1;1H\x1b[0ma\x1b[1;3H\x1b[0;31mred"),
            "{colored:?}"
        );
        let plain = drawn(false);
        assert!(plain.contains("\x1b[1;1H\x1b[0ma\x1b[1;3Hred"), "{plain:?}");
        assert!(!plain.contains(";31"), "{plain:?}");
    }
}
