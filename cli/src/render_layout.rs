/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{render::ColorTheme, render_util::RenderUtil, selection::RowWrapInfo};
use ratatui::{style::Style, text::Line};

/// A duration line whose text must be refreshed while an operation runs.
pub struct LiveTimer {
    pub line_index: usize,
    pub start_time: u64,
    pub style: Style,
}

pub struct HistoryLayout {
    pub lines: Vec<Line<'static>>,
    pub visible_lines: Vec<Line<'static>>,
    pub line_starts: Vec<usize>,
    pub live_timers: Vec<LiveTimer>,
    pub last_width: u16,
    pub last_block_start: usize,
    pub dirty_from: Option<usize>,
}

impl Default for HistoryLayout {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            visible_lines: Vec::new(),
            line_starts: Vec::new(),
            live_timers: Vec::new(),
            last_width: 0,
            last_block_start: 0,
            dirty_from: Some(0),
        }
    }
}

impl HistoryLayout {
    pub fn clear(&mut self) {
        self.lines.clear();
        self.visible_lines.clear();
        self.line_starts.clear();
        self.live_timers.clear();
        self.last_block_start = 0;
        self.dirty_from = Some(0);
    }

    pub fn reset_lines(&mut self) {
        self.lines.clear();
        self.lines.push(Line::from(""));
        self.live_timers.clear();
        self.visible_lines.clear();
        self.line_starts.clear();
    }

    pub fn invalidate_from(&mut self, message_index: usize) {
        match &mut self.dirty_from {
            Some(dirty_from) => *dirty_from = (*dirty_from).min(message_index),
            None => self.dirty_from = Some(message_index),
        }
    }

    pub fn update_dirty_from(&mut self, text_width: u16, theme: &ColorTheme) -> Option<usize> {
        if self.last_width != text_width {
            self.dirty_from = Some(0);
        }

        if self.dirty_from.is_none() {
            self.refresh_live_timers(text_width, theme);
        }

        self.dirty_from
    }

    fn refresh_live_timers(&mut self, width: u16, theme: &ColorTheme) {
        if self.live_timers.is_empty() {
            return;
        }
        let timers: Vec<_> = self
            .live_timers
            .iter()
            .map(|timer| (timer.line_index, timer.start_time, timer.style))
            .collect();
        for (line_index, start_time, style) in &timers {
            if let Some(line) = self.lines.get_mut(*line_index) {
                *line = RenderUtil::duration_line(*start_time, *style);
            }
        }
        for (line_index, _, _) in timers {
            self.replace_visual_history_line(line_index, width, theme);
        }
    }

    pub fn append_visual_history(&mut self, source_start: usize, width: u16, theme: &ColorTheme) {
        for line in self.lines.iter().skip(source_start) {
            self.line_starts.push(self.visible_lines.len());
            let lines = self.visualize_history_line(line, width, theme);
            self.visible_lines.extend(lines);
        }
    }

    fn replace_visual_history_line(&mut self, source_index: usize, width: u16, theme: &ColorTheme) {
        let Some(&visual_start) = self.line_starts.get(source_index) else {
            return;
        };
        let visual_end = self
            .line_starts
            .get(source_index + 1)
            .copied()
            .unwrap_or(self.visible_lines.len());
        let lines = self.visualize_history_line(&self.lines[source_index], width, theme);
        let new_len = lines.len();
        let old_len = visual_end - visual_start;
        self.visible_lines.splice(visual_start..visual_end, lines);

        let delta = new_len as isize - old_len as isize;
        if delta != 0 {
            for start in self.line_starts.iter_mut().skip(source_index + 1) {
                *start = (*start as isize + delta) as usize;
            }
        }
    }

    pub fn visible_history_lines(
        &self,
        lines: &[Line<'static>],
        width: u16,
        start: usize,
        height: usize,
        theme: &ColorTheme,
    ) -> (Vec<Line<'static>>, Vec<RowWrapInfo>) {
        let mut visible = Vec::with_capacity(height);
        let mut row_infos = Vec::with_capacity(height);
        let mut wrapped_index = 0usize;
        let end = start.saturating_add(height);

        for (index, line) in lines.iter().enumerate() {
            let wrapped_lines = self.visualize_history_line(line, width, theme);
            for wrapped in wrapped_lines {
                let row_info = RowWrapInfo::new(&wrapped, index);
                if Self::push_visible_line(
                    &mut visible,
                    &mut row_infos,
                    wrapped,
                    row_info,
                    &mut wrapped_index,
                    start,
                    end,
                ) {
                    return (visible, row_infos);
                }
            }
        }

        (visible, row_infos)
    }

    fn visualize_history_line(
        &self,
        line: &Line<'static>,
        width: u16,
        theme: &ColorTheme,
    ) -> Vec<Line<'static>> {
        if RowWrapInfo::is_prompt_line(line) {
            let content_line = RowWrapInfo::strip_prompt_prefix(line);
            let mut lines = Vec::new();
            for wrapped in RenderUtil::wrap_line(&content_line, width.saturating_sub(3).max(1)) {
                lines.push(theme.format_user_prompt_line(wrapped, width));
            }
            lines
        } else {
            RenderUtil::wrap_line(line, width)
        }
    }

    fn push_visible_line(
        visible: &mut Vec<Line<'static>>,
        row_infos: &mut Vec<RowWrapInfo>,
        line: Line<'static>,
        row_info: RowWrapInfo,
        wrapped_index: &mut usize,
        start: usize,
        end: usize,
    ) -> bool {
        if *wrapped_index >= start && *wrapped_index < end {
            visible.push(line);
            row_infos.push(row_info);
        }
        *wrapped_index += 1;
        *wrapped_index >= end
    }

    pub fn begin_message_block(&mut self, removed_padding: bool) {
        self.last_block_start = self.lines.len().saturating_sub(removed_padding as usize);
    }

    pub fn truncate_last_block(&mut self) {
        self.lines.truncate(self.last_block_start);
        self.live_timers
            .retain(|t| t.line_index < self.last_block_start);
        self.truncate_visual_history(self.last_block_start);
    }

    pub fn truncate_visual_history(&mut self, source_start: usize) {
        let visual_start = self
            .line_starts
            .get(source_start)
            .copied()
            .unwrap_or(self.visible_lines.len());
        self.visible_lines.truncate(visual_start);
        self.line_starts.truncate(source_start);
    }
}
