/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crossterm::{clipboard::CopyToClipboard, event::MouseEvent, execute};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
};
use std::io::{self, Write};

use crate::render_util::USER_PROMPT_BAR;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HistoryPoint {
    pub row: u16,
    pub col: u16,
}

impl HistoryPoint {
    pub fn normalize(first: HistoryPoint, second: HistoryPoint) -> (HistoryPoint, HistoryPoint) {
        if (first.row, first.col) <= (second.row, second.col) {
            (first, second)
        } else {
            (second, first)
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RowWrapInfo {
    pub index: usize,
    pub prefix: usize,
    pub content: String,
}

impl RowWrapInfo {
    pub fn new(line: &Line<'_>, index: usize) -> Self {
        if Self::is_prompt_line(line) {
            let content = Self::line_plain_text(&Self::strip_prompt_prefix(line));
            Self {
                index,
                prefix: 2,
                content,
            }
        } else {
            Self {
                index,
                prefix: 0,
                content: Self::line_plain_text(line),
            }
        }
    }

    fn line_plain_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    pub fn is_prompt_line(line: &Line<'_>) -> bool {
        line.spans
            .first()
            .map(|span| span.content == USER_PROMPT_BAR)
            .unwrap_or(false)
    }

    pub fn strip_prompt_prefix(line: &Line<'_>) -> Line<'static> {
        let spans: Vec<_> = if line
            .spans
            .first()
            .is_some_and(|s| s.content == USER_PROMPT_BAR)
        {
            line.spans[1..]
                .iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect()
        } else {
            line.spans
                .iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect()
        };
        let mut result = Line::from(spans);
        result.style = line.style;
        result.alignment = line.alignment;
        result
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HistorySelection {
    pub anchor: HistoryPoint,
    pub focus: HistoryPoint,
    pub active: bool,
}

impl HistorySelection {
    pub fn new(point: HistoryPoint) -> Self {
        Self {
            anchor: point,
            focus: point,
            active: true,
        }
    }

    pub fn normalized(&self) -> (HistoryPoint, HistoryPoint) {
        HistoryPoint::normalize(self.anchor, self.focus)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.focus
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HistoryViewport {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl HistoryViewport {
    pub fn point_for(&self, column: u16, row: u16, visible_rows: usize) -> Option<HistoryPoint> {
        if column < self.x || row < self.y {
            return None;
        }
        let col = column - self.x;
        let row = row - self.y;
        if col >= self.width || row >= self.height || row as usize >= visible_rows {
            return None;
        }
        Some(HistoryPoint { row, col })
    }
}

#[derive(Default)]
pub struct Selection {
    pub lines: Vec<Line<'static>>,
    pub row_info: Vec<RowWrapInfo>,
    pub viewport: HistoryViewport,
    pub selection: Option<HistorySelection>,
}

impl Selection {
    pub fn highlight(
        &mut self,
        lines: Vec<Line<'static>>,
        row_info: Vec<RowWrapInfo>,
        area: Rect,
        text_width: u16,
        text_height: u16,
        color: Color,
    ) -> Vec<Line<'static>> {
        self.viewport.x = area.x.saturating_add(1);
        self.viewport.y = area.y.saturating_add(1);
        self.viewport.width = text_width;
        self.viewport.height = text_height;
        self.lines = lines.clone();
        self.row_info = row_info;

        let Some(selection) = self.selection.as_ref() else {
            return lines;
        };
        let (start, end) = selection.normalized();
        if start == end {
            return lines;
        }

        lines
            .into_iter()
            .enumerate()
            .map(|(row, line)| highlight_history_line(line, row, start, end, color))
            .collect()
    }

    pub fn mouse_down(&mut self, mouse: MouseEvent) {
        if let Some(point) = self.get_point(mouse) {
            self.selection = Some(HistorySelection::new(point));
        } else {
            self.selection = None;
        }
    }

    pub fn mouse_drag(&mut self, mouse: MouseEvent) {
        let Some(point) = self.get_point(mouse) else {
            return;
        };
        if let Some(selection) = &mut self.selection
            && selection.active
        {
            selection.focus = point;
        }
    }

    pub fn mouse_up(&mut self, mouse: MouseEvent) -> Option<String> {
        let point = self.get_point(mouse);

        // Update the selection if it exists
        if let Some(selection) = &mut self.selection {
            if let Some(point) = point {
                selection.focus = point;
            }
            selection.active = false;
        }

        let Some(text) = self.selected_text() else {
            self.selection = None;
            return None;
        };

        match execute!(
            io::stdout(),
            CopyToClipboard::to_clipboard_from(text.as_str())
        ) {
            Ok(()) => {
                let _ = io::stdout().flush();
                Some("Copied selection".to_string())
            }
            Err(err) => Some(format!("Copy failed: {}", err)),
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        let Some(selection) = &self.selection else {
            return None;
        };

        let (start, end) = selection.normalized();
        if start == end || start.row as usize >= self.lines.len() {
            return None;
        }

        let last_row = (end.row as usize)
            .min(self.lines.len().saturating_sub(1))
            .min(self.row_info.len().saturating_sub(1));
        let mut result = String::new();
        let mut prev_index: Option<usize> = None;

        for row in start.row as usize..=last_row {
            let info = &self.row_info[row];
            let line_len = info.content.chars().count();
            let prefix = info.prefix;

            let from = if row == start.row as usize {
                let adjusted = (start.col as usize).saturating_sub(prefix);
                adjusted.min(line_len)
            } else {
                0
            };

            let to = if row == end.row as usize {
                let adjusted = (end.col as usize).saturating_sub(prefix);
                adjusted.min(line_len)
            } else {
                line_len
            };

            if prev_index != Some(info.index) {
                if prev_index.is_some() {
                    result.push('\n');
                }
                prev_index = Some(info.index);
            }

            if from < to {
                let piece = char_slice(&info.content, from, to);
                result.push_str(&piece);
            }
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    fn get_point(&self, mouse: MouseEvent) -> Option<HistoryPoint> {
        self.viewport
            .point_for(mouse.column, mouse.row, self.lines.len())
    }
}

fn highlight_history_line(
    line: Line<'static>,
    row: usize,
    start: HistoryPoint,
    end: HistoryPoint,
    color: Color,
) -> Line<'static> {
    if row < start.row as usize || row > end.row as usize {
        return line;
    }

    let line_len = line.width();
    let from = if row == start.row as usize {
        (start.col as usize).min(line_len)
    } else {
        0
    };
    let to = if row == end.row as usize {
        (end.col as usize).min(line_len)
    } else {
        line_len
    };
    if from >= to {
        return line;
    }

    let mut highlighted = Line::from(highlight_spans(line.spans, from, to, color));
    highlighted.style = line.style;
    highlighted.alignment = line.alignment;
    highlighted
}

fn highlight_spans(
    spans: Vec<Span<'static>>,
    from: usize,
    to: usize,
    color: Color,
) -> Vec<Span<'static>> {
    let mut result = Vec::new();
    let mut offset = 0usize;
    let selection_style = Style::default().bg(color).fg(Color::Black);

    for span in spans {
        let len = span.content.chars().count();
        let span_start = offset;
        let span_end = offset.saturating_add(len);
        offset = span_end;

        if span_end <= from || span_start >= to {
            result.push(span);
            continue;
        }

        let select_from = from.saturating_sub(span_start).min(len);
        let select_to = to.saturating_sub(span_start).min(len);
        let content = span.content.to_string();

        if select_from > 0 {
            result.push(Span::styled(
                char_slice(&content, 0, select_from),
                span.style,
            ));
        }
        if select_from < select_to {
            result.push(Span::styled(
                char_slice(&content, select_from, select_to),
                span.style.patch(selection_style),
            ));
        }
        if select_to < len {
            result.push(Span::styled(
                char_slice(&content, select_to, len),
                span.style,
            ));
        }
    }

    result
}

fn char_slice(text: &str, from: usize, to: usize) -> String {
    text.chars()
        .skip(from)
        .take(to.saturating_sub(from))
        .collect()
}
