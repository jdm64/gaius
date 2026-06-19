/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{input::InputMode, render::Render, tui::TuiApp};
use ratatui::{
    Frame,
    layout::{HorizontalAlignment, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

const INPUT_PROMPT_PREFIX: &str = "> ";

impl Render {
    pub fn draw_input(
        &self,
        app: &TuiApp,
        frame: &mut Frame<'_>,
        area: Rect,
        lines: Vec<Line<'static>>,
        input_start_line: usize,
        width: usize,
    ) {
        let mut block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(self.theme.inputbox))
            .padding(Padding::horizontal(1));
        if !app.status.is_empty() {
            if matches!(&app.mode, InputMode::Question { .. }) {
                block = block
                    .title_bottom(" Waiting for user... ".to_string())
                    .title_alignment(HorizontalAlignment::Right);
            } else {
                block = block.title(format!(" {} ", app.status));
            }
        }
        let input = Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: false });
        frame.render_widget(input, area);

        let cursor = app.input_cursor + 2;
        let (row, col) = (cursor / width, cursor % width);
        let cursor_x = area.x.saturating_add(2).saturating_add(col as u16);
        let cursor_y = area
            .y
            .saturating_add(1)
            .saturating_add(input_start_line as u16)
            .saturating_add(row as u16);

        frame.set_cursor_position((cursor_x, cursor_y));
    }

    pub fn input_prompt_lines(&self, input: String, width: u16) -> Vec<Line<'static>> {
        let line = Line::from(vec![
            Span::raw(INPUT_PROMPT_PREFIX),
            Span::raw(input),
            Span::raw(" "),
        ]);
        Self::wrap_line(&line, width)
    }

    pub fn question_lines(&self, mode: &InputMode, width: u16) -> Vec<Line<'static>> {
        let InputMode::Question {
            title,
            options,
            selected,
        } = mode
        else {
            return vec![];
        };

        let mut lines = Vec::new();
        let mut line_buf = String::new();
        for word in title.split_whitespace() {
            if line_buf.is_empty() {
                line_buf.push_str(word);
            } else if line_buf.len() + 1 + word.len() <= width as usize {
                line_buf.push(' ');
                line_buf.push_str(word);
            } else {
                lines.extend(Self::wrap_line(
                    &Line::raw(std::mem::take(&mut line_buf)),
                    width,
                ));
                line_buf = word.to_string();
            }
        }
        if !line_buf.is_empty() {
            lines.extend(Self::wrap_line(&Line::raw(line_buf), width));
        }

        lines.push(Line::raw(""));
        for (i, opt) in options.iter().enumerate() {
            let style = if i == *selected {
                Style::default().bg(self.theme.selected)
            } else {
                Style::default()
            };
            let content = format!("{}) {}", i + 1, opt);
            lines.extend(Self::wrap_line(&Line::styled(content, style), width));
        }
        lines.push(Line::raw(""));

        lines
    }

    pub fn question_help() -> Vec<(&'static str, &'static str)> {
        vec![
            ("Type", "add response"),
            ("Enter", "send"),
            ("Up/Down", "select"),
            ("Tab", "cancel"),
        ]
    }
}
