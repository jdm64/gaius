/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{render::ColorTheme, render::util::RenderUtil, selection::RowWrapInfo};
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
    pub timers: Vec<LiveTimer>,
    pub width: u16,
    pub block_start: usize,
    pub dirty_from: Option<usize>,
    pub scroll: u16,
    pub page_size: u16,
    pub height: u16,
    pub new_lines: u16,
}

impl Default for HistoryLayout {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            visible_lines: Vec::new(),
            line_starts: Vec::new(),
            timers: Vec::new(),
            width: 0,
            block_start: 0,
            dirty_from: Some(0),
            scroll: 0,
            page_size: 0,
            height: 0,
            new_lines: 0,
        }
    }
}

impl HistoryLayout {
    pub fn clear(&mut self) {
        self.lines.clear();
        self.visible_lines.clear();
        self.line_starts.clear();
        self.timers.clear();
        self.block_start = 0;
        self.dirty_from = Some(0);
        self.scroll = 0;
        self.page_size = 0;
        self.height = 0;
        self.new_lines = 0;
    }

    pub fn reset_lines(&mut self) {
        self.lines.clear();
        self.lines.push(Line::from(""));
        self.timers.clear();
        self.visible_lines.clear();
        self.line_starts.clear();
    }

    pub fn scroll_bottom(&mut self) {
        self.scroll = 0;
        self.new_lines = 0;
    }

    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll = self.scroll.saturating_add(amount);
    }

    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll = self.scroll.saturating_sub(amount);
        let dismissed = amount.min(self.new_lines);
        self.new_lines = self.new_lines.saturating_sub(dismissed);
    }

    pub fn scroll_size(&mut self) -> u16 {
        self.page_size.saturating_sub(1).max(1)
    }

    pub fn update_scroll_state(&mut self, height: u16) -> u16 {
        let wrapped_height = self.visible_lines.len() as u16;
        let max_scroll = wrapped_height.saturating_sub(height);
        let height_growth = if self.scroll != 0 && self.height != 0 {
            wrapped_height.saturating_sub(self.height)
        } else {
            0
        };

        if height_growth > 0 {
            self.scroll = self.scroll.saturating_add(height_growth);
        }
        self.page_size = height - 1;
        self.height = wrapped_height;
        self.scroll = self.scroll.min(max_scroll);

        self.new_lines = if self.scroll == 0 {
            0
        } else if height_growth > 0 {
            self.new_lines
                .saturating_add(height_growth)
                .min(self.scroll)
        } else {
            self.new_lines.min(self.scroll)
        };

        max_scroll
    }

    pub fn visible_cached_lines(&mut self, height: u16) -> (Vec<Line<'static>>, Vec<RowWrapInfo>) {
        let max_scroll = self.update_scroll_state(height);
        let start = max_scroll.saturating_sub(self.scroll) as usize;
        let end = start
            .saturating_add(height as usize)
            .min(self.visible_lines.len());

        let lines = self.visible_lines[start..end].to_vec();
        let row_info = lines
            .iter()
            .enumerate()
            .map(|(offset, line)| {
                let visual_index = start + offset;
                let source_index = self
                    .line_starts
                    .partition_point(|&row_start| row_start <= visual_index)
                    .saturating_sub(1);
                RowWrapInfo::new(line, source_index)
            })
            .collect();
        (lines, row_info)
    }

    pub fn update_width(&mut self, width: u16) {
        if self.width != width {
            self.height = self.visible_lines.len() as u16;
        }
        self.width = width;
        self.dirty_from = None;
    }

    pub fn invalidate_from(&mut self, message_index: usize) {
        match &mut self.dirty_from {
            Some(dirty_from) => *dirty_from = (*dirty_from).min(message_index),
            None => self.dirty_from = Some(message_index),
        }
    }

    pub fn update_dirty_from(&mut self, text_width: u16, theme: &ColorTheme) -> Option<usize> {
        if self.width != text_width {
            self.dirty_from = Some(0);
        }

        if self.dirty_from.is_none() {
            self.refresh_live_timers(text_width, theme);
        }

        self.dirty_from
    }

    fn refresh_live_timers(&mut self, width: u16, theme: &ColorTheme) {
        if self.timers.is_empty() {
            return;
        }
        let timers: Vec<_> = self
            .timers
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

    pub fn visualize_history_line(
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

    pub fn begin_message_block(&mut self, removed_padding: bool) {
        self.block_start = self.lines.len().saturating_sub(removed_padding as usize);
    }

    pub fn truncate_last_block(&mut self) {
        self.lines.truncate(self.block_start);
        self.timers.retain(|t| t.line_index < self.block_start);
        self.truncate_visual_history(self.block_start);
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
