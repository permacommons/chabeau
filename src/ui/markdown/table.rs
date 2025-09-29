use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::ui::layout::TableOverflowPolicy;
use crate::ui::span::SpanKind;
use crate::ui::theme::Theme;

type TableCell = Vec<Vec<(Span<'static>, SpanKind)>>;
type WrappedRow = Vec<TableCell>;
type WrappedTable = Vec<WrappedRow>;

/// Helper that collects markdown table events and produces themed table lines.
pub(crate) struct TableRenderer {
    rows: Vec<Vec<TableCell>>,
    current_row: Vec<TableCell>,
    current_cell: TableCell,
    in_header: bool,
}

impl TableRenderer {
    pub(crate) fn new() -> Self {
        Self {
            rows: Vec::new(),
            current_row: Vec::new(),
            current_cell: vec![Vec::new()],
            in_header: false,
        }
    }

    pub(crate) fn start_header(&mut self) {
        self.in_header = true;
    }

    pub(crate) fn end_header(&mut self) {
        self.in_header = false;
        if !self.current_row.is_empty() {
            self.rows.push(std::mem::take(&mut self.current_row));
        }
    }

    pub(crate) fn start_row(&mut self) {}

    pub(crate) fn end_row(&mut self) {
        if !self.current_row.is_empty() {
            if self.should_continue_previous_row() {
                self.merge_with_previous_row();
            } else {
                self.rows.push(std::mem::take(&mut self.current_row));
            }
        }
    }

    pub(crate) fn start_cell(&mut self) {
        self.current_cell = vec![Vec::new()];
    }

    pub(crate) fn end_cell(&mut self) {
        self.current_row
            .push(std::mem::take(&mut self.current_cell));
    }

    pub(crate) fn add_span(&mut self, span: Span<'static>, kind: SpanKind) {
        if self.current_cell.is_empty() {
            self.current_cell.push(Vec::new());
        }
        if let Some(last_line) = self.current_cell.last_mut() {
            last_line.push((span, kind));
        }
    }

    pub(crate) fn new_line_in_cell(&mut self) {
        if !self.current_cell.is_empty() {
            self.current_cell.push(Vec::new());
        }
    }

    #[cfg(test)]
    pub(crate) fn render_table_with_width(
        &self,
        theme: &Theme,
        terminal_width: Option<usize>,
    ) -> Vec<(Line<'static>, Vec<SpanKind>)> {
        self.render_table_with_width_policy(theme, terminal_width, TableOverflowPolicy::WrapCells)
    }

    pub(crate) fn render_table_with_width_policy(
        &self,
        theme: &Theme,
        terminal_width: Option<usize>,
        table_policy: TableOverflowPolicy,
    ) -> Vec<(Line<'static>, Vec<SpanKind>)> {
        let mut lines: Vec<(Line<'static>, Vec<SpanKind>)> = Vec::new();
        if self.rows.is_empty() {
            return lines;
        }

        let mut ideal_col_widths: Vec<usize> = Vec::new();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                let mut width = 0usize;
                for line in cell {
                    let line_width: usize = line
                        .iter()
                        .map(|(span, _)| UnicodeWidthStr::width(span.content.as_ref()))
                        .sum();
                    width = width.max(line_width);
                }
                if i >= ideal_col_widths.len() {
                    ideal_col_widths.push(width);
                } else if ideal_col_widths[i] < width {
                    ideal_col_widths[i] = width;
                }
            }
        }

        let col_widths =
            self.balance_column_widths(&ideal_col_widths, terminal_width, table_policy);
        let wrapped_rows = self.wrap_rows_for_rendering(&col_widths, table_policy);
        let table_style = theme.md_paragraph_style();

        if !wrapped_rows.is_empty() {
            let top_border = self.create_border_line("┌", "┐", "┬", "─", &col_widths);
            let top_line = Line::from(Span::styled(top_border, table_style));
            lines.push((top_line, vec![SpanKind::Text]));

            if let Some(header_row) = wrapped_rows.first() {
                let max_lines_in_header =
                    header_row.iter().map(|cell| cell.len()).max().unwrap_or(1);
                for line_idx in 0..max_lines_in_header {
                    let (line, kinds) = self.create_content_line_with_spans(
                        header_row,
                        &col_widths,
                        line_idx,
                        table_style,
                    );
                    lines.push((line, kinds));
                }
                if wrapped_rows.len() > 1 {
                    let header_sep = self.create_separator_line("├", "┤", "┼", "─", &col_widths);
                    let sep_line = Line::from(Span::styled(header_sep, table_style));
                    lines.push((sep_line, vec![SpanKind::Text]));
                }
            }

            for row in wrapped_rows.iter().skip(1) {
                for line_idx in 0..row.iter().map(|cell| cell.len()).max().unwrap_or(1) {
                    let (line, kinds) = self.create_content_line_with_spans(
                        row,
                        &col_widths,
                        line_idx,
                        table_style,
                    );
                    lines.push((line, kinds));
                }
            }

            let bottom_border = self.create_border_line("└", "┘", "┴", "─", &col_widths);
            let bottom_line = Line::from(Span::styled(bottom_border, table_style));
            lines.push((bottom_line, vec![SpanKind::Text]));
        }

        lines
    }

    pub(crate) fn balance_column_widths(
        &self,
        ideal_widths: &[usize],
        terminal_width: Option<usize>,
        _table_policy: TableOverflowPolicy,
    ) -> Vec<usize> {
        if ideal_widths.is_empty() {
            return Vec::new();
        }

        let num_cols = ideal_widths.len();
        const MIN_COL_WIDTH: usize = 8;
        let col_widths: Vec<usize> = ideal_widths.iter().map(|&w| w.max(MIN_COL_WIDTH)).collect();

        let Some(term_width) = terminal_width else {
            return col_widths;
        };

        let table_overhead = num_cols * 2 + (num_cols + 1);
        if term_width <= table_overhead {
            return vec![MIN_COL_WIDTH; num_cols];
        }

        let available_width = term_width - table_overhead;
        let total_ideal_width: usize = ideal_widths.iter().sum();
        if total_ideal_width <= available_width {
            let mut widths: Vec<usize> = ideal_widths.to_vec();
            let mut min_word_widths = vec![MIN_COL_WIDTH; num_cols];
            for row in &self.rows {
                for (i, cell) in row.iter().enumerate() {
                    if i < min_word_widths.len() {
                        for line in cell {
                            for (span, _) in line {
                                for word in span.content.split_whitespace() {
                                    let ww = UnicodeWidthStr::width(word);
                                    if ww <= 30 && min_word_widths[i] < ww {
                                        min_word_widths[i] = ww;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            for i in 0..widths.len() {
                if widths[i] < MIN_COL_WIDTH {
                    widths[i] = MIN_COL_WIDTH;
                }
                if widths[i] < min_word_widths[i] {
                    widths[i] = min_word_widths[i];
                }
            }
            return widths;
        }

        let mut min_word_widths = vec![MIN_COL_WIDTH; num_cols];
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < min_word_widths.len() {
                    for line in cell {
                        for (span, _) in line {
                            let words = span.content.split_whitespace();
                            for word in words {
                                let word_width = UnicodeWidthStr::width(word);
                                if word_width <= 30 {
                                    min_word_widths[i] = min_word_widths[i].max(word_width);
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut base_widths = min_word_widths.clone();
        for w in &mut base_widths {
            if *w < MIN_COL_WIDTH {
                *w = MIN_COL_WIDTH;
            }
        }

        let total_min_width: usize = base_widths.iter().sum();
        if total_min_width > available_width {
            return min_word_widths;
        }

        let extra_space = available_width - total_min_width;
        let desired_gains: Vec<usize> = ideal_widths
            .iter()
            .zip(&base_widths)
            .map(|(&ideal, &base)| ideal.saturating_sub(base))
            .collect();
        let total_desired: usize = desired_gains.iter().sum();
        let mut final_widths = base_widths.clone();
        if total_desired == 0 {
            return final_widths;
        }
        let mut allocated = 0usize;
        for i in 0..final_widths.len() {
            let prop = desired_gains[i] as f64 / total_desired as f64;
            let mut add = (extra_space as f64 * prop).floor() as usize;
            let cap = ideal_widths[i].saturating_sub(final_widths[i]);
            if add > cap {
                add = cap;
            }
            final_widths[i] += add;
            allocated += add;
        }
        let mut rem = extra_space.saturating_sub(allocated);
        if rem > 0 {
            for i in 0..final_widths.len() {
                if rem == 0 {
                    break;
                }
                let cap = ideal_widths[i].saturating_sub(final_widths[i]);
                if cap > 0 {
                    final_widths[i] += 1;
                    rem -= 1;
                }
            }
        }
        final_widths
    }

    pub(crate) fn wrap_spans_to_width(
        &self,
        spans: &[(Span<'static>, SpanKind)],
        max_width: usize,
        _table_policy: TableOverflowPolicy,
    ) -> Vec<Vec<(Span<'static>, SpanKind)>> {
        if spans.is_empty() {
            return vec![Vec::new()];
        }

        #[derive(Clone, Copy, PartialEq, Eq)]
        enum TokKind {
            Space,
            BreakChar,
            Word,
        }

        #[derive(Clone)]
        struct Tok {
            text: String,
            style: Style,
            kind: TokKind,
            width: usize,
            span_kind: SpanKind,
        }

        fn ch_width(ch: char) -> usize {
            UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]))
        }

        fn str_width(s: &str) -> usize {
            UnicodeWidthStr::width(s)
        }

        fn is_break_char(ch: char) -> bool {
            matches!(ch, '-' | '‐' | '–' | '—' | '/')
        }

        fn tokenize(text: &str, style: Style, span_kind: SpanKind) -> Vec<Tok> {
            let mut toks: Vec<Tok> = Vec::new();
            let mut buf = String::new();
            let mut mode: Option<TokKind> = None;
            for ch in text.chars() {
                let kind = if ch.is_whitespace() {
                    TokKind::Space
                } else if is_break_char(ch) {
                    TokKind::BreakChar
                } else {
                    TokKind::Word
                };
                match (mode, kind) {
                    (Some(TokKind::Space), TokKind::Space) => buf.push(ch),
                    (Some(TokKind::Word), TokKind::Word) => buf.push(ch),
                    (Some(prev), k) if prev != k => {
                        if !buf.is_empty() {
                            let w = str_width(&buf);
                            toks.push(Tok {
                                text: std::mem::take(&mut buf),
                                style,
                                kind: prev,
                                width: w,
                                span_kind: span_kind.clone(),
                            });
                        }
                        if k == TokKind::BreakChar {
                            let s = ch.to_string();
                            toks.push(Tok {
                                width: ch_width(ch),
                                text: s,
                                style,
                                kind: TokKind::BreakChar,
                                span_kind: span_kind.clone(),
                            });
                            mode = None;
                        } else {
                            buf.push(ch);
                            mode = Some(k);
                        }
                    }
                    (None, TokKind::BreakChar) => {
                        let s = ch.to_string();
                        toks.push(Tok {
                            width: ch_width(ch),
                            text: s,
                            style,
                            kind: TokKind::BreakChar,
                            span_kind: span_kind.clone(),
                        });
                        mode = None;
                    }
                    (None, k) => {
                        buf.push(ch);
                        mode = Some(k);
                    }
                    _ => unreachable!(),
                }
            }
            if !buf.is_empty() {
                let k = mode.unwrap_or(TokKind::Word);
                let w = str_width(&buf);
                toks.push(Tok {
                    text: buf,
                    style,
                    kind: k,
                    width: w,
                    span_kind: span_kind.clone(),
                });
            }
            toks
        }

        let mut all_toks: Vec<Tok> = Vec::new();
        for (span, span_kind) in spans {
            if span.content.is_empty() {
                continue;
            }
            let mut toks = tokenize(span.content.as_ref(), span.style, span_kind.clone());
            all_toks.append(&mut toks);
        }

        if all_toks.is_empty() {
            return vec![Vec::new()];
        }

        let mut out_lines: Vec<Vec<(Span<'static>, SpanKind)>> = Vec::new();
        let mut cur: Vec<Tok> = Vec::new();
        let mut cur_width: usize = 0;
        let mut last_break_idx: Option<usize> = None;

        let mut i = 0usize;
        while i < all_toks.len() {
            let tok = all_toks[i].clone();
            let w = tok.width;
            let fits = cur_width + w <= max_width;
            if fits {
                if matches!(tok.kind, TokKind::Space) && cur.is_empty() {
                    i += 1;
                    continue;
                }
                cur_width += w;
                if matches!(tok.kind, TokKind::Space | TokKind::BreakChar) {
                    last_break_idx = Some(cur.len() + 1);
                }
                cur.push(tok);
                i += 1;
                continue;
            }

            if let Some(br) = last_break_idx {
                let mut left = cur[..br.min(cur.len())].to_vec();
                while left
                    .last()
                    .map(|t| t.kind == TokKind::Space)
                    .unwrap_or(false)
                {
                    let last = left.pop().unwrap();
                    cur_width = cur_width.saturating_sub(last.width);
                }

                if !left.is_empty() {
                    let spans_line: Vec<(Span<'static>, SpanKind)> = left
                        .into_iter()
                        .map(|t| (Span::styled(t.text, t.style), t.span_kind))
                        .collect();
                    out_lines.push(spans_line);
                }

                let mut right: Vec<Tok> = cur[br.min(cur.len())..].to_vec();
                while right
                    .first()
                    .map(|t| t.kind == TokKind::Space)
                    .unwrap_or(false)
                {
                    let first = right.remove(0);
                    let _ = first;
                }
                cur = right;
                cur_width = cur.iter().map(|t| t.width).sum();
                last_break_idx = None;
                continue;
            }

            if matches!(tok.kind, TokKind::Space) {
                if !cur.is_empty() {
                    let line_spans: Vec<(Span<'static>, SpanKind)> = cur
                        .drain(..)
                        .map(|t| (Span::styled(t.text, t.style), t.span_kind))
                        .collect();
                    out_lines.push(line_spans);
                }
                cur_width = 0;
                last_break_idx = None;
                i += 1;
                continue;
            }

            let mut acc = 0usize;
            let mut cut = 0usize;
            for (pos, ch) in tok.text.char_indices() {
                let cw = ch_width(ch);
                if cur_width + acc + cw > max_width {
                    break;
                }
                acc += cw;
                cut = pos + ch.len_utf8();
            }

            if cut == 0 {
                if !cur.is_empty() {
                    let line_spans: Vec<(Span<'static>, SpanKind)> = cur
                        .drain(..)
                        .map(|t| (Span::styled(t.text, t.style), t.span_kind))
                        .collect();
                    out_lines.push(line_spans);
                }
                cur_width = 0;
                last_break_idx = None;
                if matches!(tok.kind, TokKind::Space) {
                    i += 1;
                    continue;
                }
                let mut acc2 = 0usize;
                let mut cut2 = 0usize;
                for (pos, ch) in tok.text.char_indices() {
                    let cw = ch_width(ch);
                    if acc2 + cw > max_width {
                        break;
                    }
                    acc2 += cw;
                    cut2 = pos + ch.len_utf8();
                }
                if cut2 == 0 {
                    cur_width = tok.width;
                    cur.push(tok);
                    i += 1;
                } else {
                    let left_text = tok.text[..cut2].to_string();
                    let right_text = tok.text[cut2..].to_string();
                    let left_tok = Tok {
                        width: str_width(&left_text),
                        text: left_text,
                        style: tok.style,
                        kind: TokKind::Word,
                        span_kind: tok.span_kind.clone(),
                    };
                    let right_tok = Tok {
                        width: str_width(&right_text),
                        text: right_text,
                        style: tok.style,
                        kind: TokKind::Word,
                        span_kind: tok.span_kind.clone(),
                    };
                    cur.push(left_tok);
                    let line_spans: Vec<(Span<'static>, SpanKind)> = cur
                        .drain(..)
                        .map(|t| (Span::styled(t.text, t.style), t.span_kind))
                        .collect();
                    out_lines.push(line_spans);
                    cur_width = 0;
                    last_break_idx = None;
                    all_toks[i] = right_tok;
                }
            } else {
                let left_text = tok.text[..cut].to_string();
                let right_text = tok.text[cut..].to_string();
                let left_tok = Tok {
                    width: str_width(&left_text),
                    text: left_text,
                    style: tok.style,
                    kind: TokKind::Word,
                    span_kind: tok.span_kind.clone(),
                };
                let right_tok = Tok {
                    width: str_width(&right_text),
                    text: right_text,
                    style: tok.style,
                    kind: TokKind::Word,
                    span_kind: tok.span_kind.clone(),
                };
                cur.push(left_tok);
                let line_spans: Vec<(Span<'static>, SpanKind)> = cur
                    .drain(..)
                    .map(|t| (Span::styled(t.text, t.style), t.span_kind))
                    .collect();
                out_lines.push(line_spans);
                cur_width = 0;
                last_break_idx = None;
                all_toks[i] = right_tok;
            }
        }

        while cur
            .last()
            .map(|t| t.kind == TokKind::Space)
            .unwrap_or(false)
        {
            let last = cur.pop().unwrap();
            cur_width = cur_width.saturating_sub(last.width);
        }
        if !cur.is_empty() {
            let line_spans: Vec<(Span<'static>, SpanKind)> = cur
                .into_iter()
                .map(|t| (Span::styled(t.text, t.style), t.span_kind))
                .collect();
            out_lines.push(line_spans);
        }

        if out_lines.is_empty() {
            out_lines.push(Vec::new());
        }

        out_lines
    }

    fn should_continue_previous_row(&self) -> bool {
        if self.current_row.len() < 2 {
            return false;
        }
        if self.current_row[0].iter().all(|line| line.is_empty()) {
            return true;
        }
        false
    }

    fn merge_with_previous_row(&mut self) {
        if let Some(last_row) = self.rows.last_mut() {
            for (i, cell) in self.current_row.iter().enumerate() {
                if let Some(prev_cell) = last_row.get_mut(i) {
                    prev_cell.extend(cell.clone());
                }
            }
        }
        self.current_row.clear();
    }

    fn wrap_rows_for_rendering(
        &self,
        col_widths: &[usize],
        table_policy: TableOverflowPolicy,
    ) -> WrappedTable {
        let mut wrapped_rows: WrappedTable = Vec::new();
        for row in &self.rows {
            let mut wrapped_row: WrappedRow = Vec::new();
            for (col_idx, cell) in row.iter().enumerate() {
                let col_width = col_widths
                    .get(col_idx)
                    .copied()
                    .unwrap_or_else(|| UnicodeWidthStr::width(" "));
                let mut wrapped_cell: TableCell = Vec::new();
                let effective_width = col_width.max(1);
                for line in cell {
                    let wrapped = self.wrap_spans_to_width(line, effective_width, table_policy);
                    wrapped_cell.extend(wrapped);
                }
                wrapped_row.push(wrapped_cell);
            }
            wrapped_rows.push(wrapped_row);
        }
        wrapped_rows
    }

    fn create_border_line(
        &self,
        left: &str,
        right: &str,
        mid: &str,
        fill: &str,
        col_widths: &[usize],
    ) -> String {
        let mut line = String::new();
        line.push_str(left);
        for (i, &width) in col_widths.iter().enumerate() {
            line.push_str(&fill.repeat(width + 2));
            if i < col_widths.len() - 1 {
                line.push_str(mid);
            }
        }
        line.push_str(right);
        line
    }

    fn create_separator_line(
        &self,
        left: &str,
        right: &str,
        mid: &str,
        fill: &str,
        col_widths: &[usize],
    ) -> String {
        let mut line = String::new();
        line.push_str(left);
        for (i, &width) in col_widths.iter().enumerate() {
            line.push_str(&fill.repeat(width + 2));
            if i < col_widths.len() - 1 {
                line.push_str(mid);
            }
        }
        line.push_str(right);
        line
    }

    fn create_content_line_with_spans(
        &self,
        row: &[TableCell],
        col_widths: &[usize],
        line_idx: usize,
        style: Style,
    ) -> (Line<'static>, Vec<SpanKind>) {
        let mut spans = Vec::new();
        let mut kinds = Vec::new();

        spans.push(Span::styled("│", style));
        kinds.push(SpanKind::Text);

        for (i, width) in col_widths.iter().enumerate() {
            spans.push(Span::raw(" "));
            kinds.push(SpanKind::Text);

            let cell_spans = row
                .get(i)
                .and_then(|cell| cell.get(line_idx))
                .cloned()
                .unwrap_or_default();
            let mut cell_text_len: usize = cell_spans
                .iter()
                .map(|(span, _)| UnicodeWidthStr::width(span.content.as_ref()))
                .sum();
            let mut rendered_cell = cell_spans;

            if cell_text_len > *width {
                let mut clipped: Vec<(Span<'static>, SpanKind)> = Vec::new();
                let mut used = 0usize;
                for (span, kind) in rendered_cell.into_iter() {
                    let span_width = UnicodeWidthStr::width(span.content.as_ref());
                    if used + span_width <= *width {
                        used += span_width;
                        clipped.push((span, kind));
                    } else if used < *width {
                        let remaining = *width - used;
                        let clipped_text =
                            self.clip_text_to_width(span.content.as_ref(), remaining);
                        if !clipped_text.is_empty() {
                            clipped.push((Span::styled(clipped_text, span.style), kind));
                            used += remaining;
                        }
                        break;
                    } else {
                        break;
                    }
                }
                rendered_cell = clipped;
                cell_text_len = used;
            }

            if cell_text_len < *width {
                rendered_cell.push((Span::raw(" ".repeat(width - cell_text_len)), SpanKind::Text));
            }

            for (span, kind) in rendered_cell.into_iter() {
                spans.push(span);
                kinds.push(kind);
            }

            spans.push(Span::raw(" "));
            kinds.push(SpanKind::Text);
            spans.push(Span::styled("│", style));
            kinds.push(SpanKind::Text);
        }

        (Line::from(spans), kinds)
    }

    fn clip_text_to_width(&self, text: &str, max_width: usize) -> String {
        let mut result = String::new();
        let mut current_width = 0;

        for ch in text.chars() {
            let ch_width = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]));
            if current_width + ch_width > max_width {
                break;
            }
            result.push(ch);
            current_width += ch_width;
        }

        result
    }
}
