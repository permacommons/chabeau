use std::collections::{HashMap, HashSet};
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
    cached_cells: HashMap<(u16, u16), Cell>,
    prev_links: LinkEvents,
}

#[derive(Debug, Clone, Default)]
struct LinkEvents {
    starts: HashMap<(u16, u16), Vec<String>>,
    ends: HashMap<(u16, u16), Vec<String>>,
    spans: HashSet<LinkSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LinkSpan {
    href: String,
    end: (u16, u16),
}

impl<W> OscBackend<W>
where
    W: Write,
{
    pub fn new(writer: W) -> Self {
        Self {
            inner: CrosstermBackend::new(writer),
            cached_cells: HashMap::new(),
            prev_links: LinkEvents {
                starts: HashMap::new(),
                ends: HashMap::new(),
                spans: HashSet::new(),
            },
        }
    }

    fn hyperlink_events(&self) -> LinkEvents {
        let state = take_render_state();
        let mut starts: HashMap<(u16, u16), Vec<String>> = HashMap::new();
        let mut ends: HashMap<(u16, u16), Vec<String>> = HashMap::new();
        let mut spans: HashSet<LinkSpan> = HashSet::new();

        for span in state.spans {
            let start = (*span.x_range.start(), span.y);
            let end = (*span.x_range.end(), span.y);
            let href = span.href.href().to_string();
            starts.entry(start).or_default().push(href.clone());
            ends.entry(end).or_default().push(href.clone());
            spans.insert(LinkSpan { href, end });
        }

        LinkEvents {
            starts,
            ends,
            spans,
        }
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
        let prev_links = std::mem::take(&mut self.prev_links);

        let mut forced_cells: HashSet<(u16, u16)> = HashSet::new();
        let mut pre_prefix_closures: HashMap<(u16, u16), usize> = HashMap::new();
        let mut stale_closure_counts: HashMap<(u16, u16), usize> = HashMap::new();

        let mut stale_spans: Vec<LinkSpan> = prev_links
            .spans
            .difference(&events.spans)
            .cloned()
            .collect();
        stale_spans.sort_by(|a, b| (a.end.1, a.end.0).cmp(&(b.end.1, b.end.0)));
        for span in &stale_spans {
            *stale_closure_counts.entry(span.end).or_insert(0) += 1;
        }

        for position in prev_links
            .starts
            .keys()
            .chain(events.starts.keys())
            .copied()
        {
            if prev_links.starts.get(&position) != events.starts.get(&position) {
                forced_cells.insert(position);
            }
        }

        for position in prev_links.ends.keys().chain(events.ends.keys()).copied() {
            if prev_links.ends.get(&position) != events.ends.get(&position) {
                forced_cells.insert(position);
            }
        }

        for (position, prev_hrefs) in &prev_links.ends {
            let mut prev_counts: HashMap<&str, usize> = HashMap::new();
            for href in prev_hrefs {
                *prev_counts.entry(href.as_str()).or_insert(0) += 1;
            }

            if let Some(next_hrefs) = events.ends.get(position) {
                for href in next_hrefs {
                    if let Some(count) = prev_counts.get_mut(href.as_str()) {
                        if *count > 0 {
                            *count -= 1;
                        }
                    }
                }
            }

            let leftover = prev_counts.values().copied().sum::<usize>();
            if leftover > 0 {
                forced_cells.insert(*position);
                pre_prefix_closures.insert(*position, leftover);
            }
        }

        for (position, count) in &stale_closure_counts {
            if let Some(existing) = pre_prefix_closures.get_mut(position) {
                if *existing > *count {
                    *existing -= *count;
                } else {
                    *existing = 0;
                }
            }
        }
        pre_prefix_closures.retain(|_, count| *count > 0);

        let mut changed_cells: Vec<(u16, u16, Cell)> = content
            .map(|(x, y, cell)| {
                let cell_clone = cell.clone();
                self.cached_cells.insert((x, y), cell_clone.clone());
                (x, y, cell_clone)
            })
            .collect();

        let mut changed_positions: HashSet<(u16, u16)> =
            changed_cells.iter().map(|(x, y, _)| (*x, *y)).collect();

        for position in forced_cells {
            if changed_positions.contains(&position) {
                continue;
            }

            if let Some(cell) = self.cached_cells.get(&position).cloned() {
                changed_cells.push((position.0, position.1, cell));
                changed_positions.insert(position);
            }
        }

        changed_cells.sort_by(|a, b| (a.1, a.0).cmp(&(b.1, b.0)));

        let mut fg = Color::Reset;
        let mut bg = Color::Reset;
        let mut modifier = Modifier::empty();
        let mut last_pos: Option<Position> = None;

        for span in &stale_spans {
            let position = Position {
                x: span.end.0,
                y: span.end.1,
            };
            if !matches!(last_pos, Some(p) if p.x == position.x && p.y == position.y) {
                queue!(self.inner, MoveTo(position.x, position.y))?;
            }
            last_pos = Some(position);
            self.queue_suffix()?;
        }

        for (x, y, cell) in changed_cells {
            if !matches!(last_pos, Some(p) if x == p.x + 1 && y == p.y) {
                queue!(self.inner, MoveTo(x, y))?;
            }
            last_pos = Some(Position { x, y });

            if let Some(count) = pre_prefix_closures.get(&(x, y)) {
                for _ in 0..*count {
                    self.queue_suffix()?;
                }
            }

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

            self.cached_cells.insert((x, y), cell.clone());
        }

        self.prev_links = events;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::osc_state::{set_render_state, OscRenderState, OscSpan};
    use crate::ui::span::LinkMeta;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::{Arc, LazyLock, Mutex};

    static TEST_RENDER_STATE_GUARD: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn cell_with_symbol(symbol: &str) -> Cell {
        let mut cell = Cell::default();
        cell.set_symbol(symbol);
        cell
    }

    #[derive(Clone)]
    struct RecordingWriter(Rc<RefCell<Vec<u8>>>);

    impl Write for RecordingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn backend_with_recorder() -> (OscBackend<RecordingWriter>, Rc<RefCell<Vec<u8>>>) {
        let storage = Rc::new(RefCell::new(Vec::new()));
        let writer = RecordingWriter(storage.clone());
        (OscBackend::new(writer), storage)
    }

    fn span(href: &str, x: u16, y: u16) -> OscSpan {
        OscSpan {
            href: Arc::new(LinkMeta::new(href)),
            y,
            x_range: x..=x,
        }
    }

    #[test]
    fn redraws_cell_when_hyperlink_changes_without_buffer_diff() {
        let _guard = TEST_RENDER_STATE_GUARD.lock().unwrap();
        let (mut backend, storage) = backend_with_recorder();
        let cell = cell_with_symbol("A");

        set_render_state(OscRenderState {
            spans: vec![span("https://old.example", 0, 0)],
        });

        let cells = [(0u16, 0u16, cell.clone())];
        backend
            .draw(cells.iter().map(|(x, y, cell)| (*x, *y, cell)))
            .unwrap();

        storage.borrow_mut().clear();

        set_render_state(OscRenderState {
            spans: vec![span("https://new.example", 0, 0)],
        });

        backend
            .draw(std::iter::empty::<(u16, u16, &Cell)>())
            .unwrap();

        let output = storage.borrow();
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("\x1b]8;;https://new.example\x1b\\"));
        assert!(output.contains("\x1b]8;;\x1b\\"));

        set_render_state(OscRenderState::default());
    }

    #[test]
    fn closes_stale_hyperlink_even_without_cell_diff() {
        let _guard = TEST_RENDER_STATE_GUARD.lock().unwrap();
        let (mut backend, storage) = backend_with_recorder();
        let cell = cell_with_symbol("A");

        set_render_state(OscRenderState {
            spans: vec![span("https://old.example", 0, 0)],
        });

        let cells = [(0u16, 0u16, cell.clone())];
        backend
            .draw(cells.iter().map(|(x, y, cell)| (*x, *y, cell)))
            .unwrap();

        storage.borrow_mut().clear();

        set_render_state(OscRenderState { spans: Vec::new() });

        backend
            .draw(std::iter::empty::<(u16, u16, &Cell)>())
            .unwrap();

        let output = storage.borrow();
        let output = String::from_utf8_lossy(&output);
        assert!(!output.contains("https://old.example"));
        assert!(output.contains("\x1b]8;;\x1b\\"));

        set_render_state(OscRenderState::default());
    }

    #[test]
    fn closes_hyperlink_removed_by_scroll_without_touching_endpoint() {
        let _guard = TEST_RENDER_STATE_GUARD.lock().unwrap();
        let (mut backend, storage) = backend_with_recorder();
        let top_cell = cell_with_symbol("A");
        let scrolled_cell = cell_with_symbol("B");

        set_render_state(OscRenderState {
            spans: vec![span("https://scroll.example", 0, 0)],
        });

        let initial_cells = [(0u16, 0u16, top_cell.clone())];
        backend
            .draw(initial_cells.iter().map(|(x, y, cell)| (*x, *y, cell)))
            .unwrap();

        storage.borrow_mut().clear();

        set_render_state(OscRenderState { spans: Vec::new() });

        let scrolled_cells = [(0u16, 1u16, scrolled_cell.clone())];
        backend
            .draw(scrolled_cells.iter().map(|(x, y, cell)| (*x, *y, cell)))
            .unwrap();

        let output = storage.borrow();
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("\x1b]8;;\x1b\\"));

        set_render_state(OscRenderState::default());
    }
}
