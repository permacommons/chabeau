use std::collections::HashMap;
use std::io::{self, Write};

use ratatui::{
    backend::{Backend, ClearType, CrosstermBackend, WindowSize},
    buffer::Cell,
    crossterm::{
        cursor::MoveTo,
        execute, queue,
        style::{
            Attribute as CAttribute, Color as CColor, Colors, Print, SetAttribute,
            SetBackgroundColor, SetColors, SetForegroundColor,
        },
        terminal::{Clear, ClearType as CrosstermClearType},
    },
    layout::{Position, Size},
    style::{Color, Modifier},
};

use crate::ui::osc_state::take_render_state;

/// Crossterm backend wrapper that injects OSC8 hyperlinks while preserving ratatui invariants.
#[derive(Debug)]
pub struct OscBackend<W: Write> {
    inner: CrosstermBackend<W>,
}

struct LinkEvents {
    starts: HashMap<(u16, u16), Vec<String>>,
    ends: HashMap<(u16, u16), Vec<String>>,
}

impl<W> OscBackend<W>
where
    W: Write,
{
    pub const fn new(writer: W) -> Self {
        Self {
            inner: CrosstermBackend::new(writer),
        }
    }

    fn hyperlink_events(&self) -> LinkEvents {
        let state = take_render_state();
        let mut starts: HashMap<(u16, u16), Vec<String>> = HashMap::new();
        let mut ends: HashMap<(u16, u16), Vec<String>> = HashMap::new();

        for span in state.spans {
            let start = (*span.x_range.start(), span.y);
            let end = (*span.x_range.end(), span.y);
            let href = span.href.href().to_string();
            starts.entry(start).or_default().push(href.clone());
            ends.entry(end).or_default().push(href);
        }

        LinkEvents { starts, ends }
    }

    fn queue_prefix(&mut self, href: &str) -> io::Result<()> {
        queue!(self.inner, Print("\x1b]8;;"))?;
        queue!(self.inner, Print(href))?;
        queue!(self.inner, Print("\x1b\\"))
    }

    fn queue_suffix(&mut self) -> io::Result<()> {
        queue!(self.inner, Print("\x1b]8;;\x1b\\"))
    }
}

impl<W> Write for OscBackend<W>
where
    W: Write,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        std::io::Write::flush(&mut self.inner)
    }
}

impl<W> Backend for OscBackend<W>
where
    W: Write,
{
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let events = self.hyperlink_events();

        let mut fg = Color::Reset;
        let mut bg = Color::Reset;
        let mut modifier = Modifier::empty();
        let mut last_pos: Option<Position> = None;

        for (x, y, cell) in content {
            if !matches!(last_pos, Some(p) if x == p.x + 1 && y == p.y) {
                queue!(self.inner, MoveTo(x, y))?;
            }
            last_pos = Some(Position { x, y });

            if let Some(hrefs) = events.starts.get(&(x, y)) {
                for href in hrefs {
                    self.queue_prefix(href)?;
                }
            }

            if cell.modifier != modifier {
                let diff = ModifierDiff {
                    from: modifier,
                    to: cell.modifier,
                };
                diff.queue(&mut self.inner)?;
                modifier = cell.modifier;
            }
            if cell.fg != fg || cell.bg != bg {
                queue!(
                    self.inner,
                    SetColors(Colors::new(cell.fg.into(), cell.bg.into()))
                )?;
                fg = cell.fg;
                bg = cell.bg;
            }
            queue!(self.inner, Print(cell.symbol()))?;

            if let Some(hrefs) = events.ends.get(&(x, y)) {
                for _ in hrefs {
                    self.queue_suffix()?;
                }
            }
        }

        queue!(
            self.inner,
            SetForegroundColor(CColor::Reset),
            SetBackgroundColor(CColor::Reset),
            SetAttribute(CAttribute::Reset),
        )
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        execute!(
            self.inner,
            Clear(match clear_type {
                ClearType::All => CrosstermClearType::All,
                ClearType::AfterCursor => CrosstermClearType::FromCursorDown,
                ClearType::BeforeCursor => CrosstermClearType::FromCursorUp,
                ClearType::CurrentLine => CrosstermClearType::CurrentLine,
                ClearType::UntilNewLine => CrosstermClearType::UntilNewLine,
            })
        )
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.inner.append_lines(n)
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        Backend::flush(&mut self.inner)
    }
}

struct ModifierDiff {
    from: Modifier,
    to: Modifier,
}

impl ModifierDiff {
    fn queue<W>(&self, mut w: W) -> io::Result<()>
    where
        W: io::Write,
    {
        let removed = self.from - self.to;
        if removed.contains(Modifier::REVERSED) {
            queue!(w, SetAttribute(CAttribute::NoReverse))?;
        }
        if removed.contains(Modifier::BOLD) {
            queue!(w, SetAttribute(CAttribute::NormalIntensity))?;
            if self.to.contains(Modifier::DIM) {
                queue!(w, SetAttribute(CAttribute::Dim))?;
            }
        }
        if removed.contains(Modifier::ITALIC) {
            queue!(w, SetAttribute(CAttribute::NoItalic))?;
        }
        if removed.contains(Modifier::UNDERLINED) {
            queue!(w, SetAttribute(CAttribute::NoUnderline))?;
        }
        if removed.contains(Modifier::DIM) {
            queue!(w, SetAttribute(CAttribute::NormalIntensity))?;
        }
        if removed.contains(Modifier::CROSSED_OUT) {
            queue!(w, SetAttribute(CAttribute::NotCrossedOut))?;
        }
        if removed.contains(Modifier::SLOW_BLINK) || removed.contains(Modifier::RAPID_BLINK) {
            queue!(w, SetAttribute(CAttribute::NoBlink))?;
        }

        let added = self.to - self.from;
        if added.contains(Modifier::REVERSED) {
            queue!(w, SetAttribute(CAttribute::Reverse))?;
        }
        if added.contains(Modifier::BOLD) {
            queue!(w, SetAttribute(CAttribute::Bold))?;
        }
        if added.contains(Modifier::ITALIC) {
            queue!(w, SetAttribute(CAttribute::Italic))?;
        }
        if added.contains(Modifier::UNDERLINED) {
            queue!(w, SetAttribute(CAttribute::Underlined))?;
        }
        if added.contains(Modifier::DIM) {
            queue!(w, SetAttribute(CAttribute::Dim))?;
        }
        if added.contains(Modifier::CROSSED_OUT) {
            queue!(w, SetAttribute(CAttribute::CrossedOut))?;
        }
        if added.contains(Modifier::SLOW_BLINK) {
            queue!(w, SetAttribute(CAttribute::SlowBlink))?;
        }
        if added.contains(Modifier::RAPID_BLINK) {
            queue!(w, SetAttribute(CAttribute::RapidBlink))?;
        }

        Ok(())
    }
}
