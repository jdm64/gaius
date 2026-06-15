/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{input::InputMode, tui::TuiApp};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};

pub const INPUT_HEIGHT: u16 = 3;

pub struct ColorTheme {
    pub header: Color,
    pub selected: Color,
    pub thinking: Color,
    pub user_bar: Color,
    pub user_box: Color,
    pub inputbox: Color,
    pub toolcall: Color,
    pub error: Color,
}

impl Default for ColorTheme {
    fn default() -> Self {
        ColorTheme {
            header: Color::Yellow,
            selected: Color::Magenta,
            thinking: Color::LightBlue,
            user_bar: Color::Magenta,
            user_box: Color::Rgb(64, 64, 64),
            inputbox: Color::Rgb(64, 0, 64),
            toolcall: Color::Cyan,
            error: Color::Red,
        }
    }
}

pub struct Render {
    pub theme: ColorTheme,
}

impl Default for Render {
    fn default() -> Self {
        Self::new()
    }
}

impl Render {
    pub fn new() -> Self {
        Render {
            theme: ColorTheme::default(),
        }
    }

    pub fn draw(&self, app: &mut TuiApp, frame: &mut Frame<'_>) {
        let area = frame.area();
        let input_width = area.width.saturating_sub(4).max(1);

        let question_lines = self.question_lines(&app.mode, input_width);
        let input_prompt = self.input_prompt_lines(app.input.clone(), input_width);
        let input_start_line = question_lines.len();

        let mut input_lines = question_lines;
        input_lines.extend(input_prompt);
        let input_height = 2 + input_lines.len() as u16;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(input_height),
                Constraint::Length(1),
            ])
            .split(area);

        frame.render_widget(Clear, area);
        self.draw_history(app, frame, chunks[0]);
        self.draw_input(
            app,
            frame,
            chunks[1],
            input_lines,
            input_start_line,
            input_width as usize,
        );

        let active_help: Option<Vec<(&'static str, &'static str)>> = match &app.mode {
            InputMode::Command { picker } => self.draw_commands(frame, chunks[1], picker),
            InputMode::Session { picker } => self.draw_sessions(frame, chunks[1], picker, false),
            InputMode::SessionRename { picker } => {
                self.draw_sessions(frame, chunks[1], picker, true)
            }
            InputMode::Models { picker } => self.draw_models(frame, chunks[1], picker),
            InputMode::AddProvider { picker } => {
                self.draw_add_provider(frame, chunks[1], picker, app.input.as_str())
            }
            InputMode::Agents { picker } => self.draw_agents(frame, chunks[1], picker),
            InputMode::Files { picker } => self.draw_files(frame, chunks[1], picker),
            InputMode::PromptInput | InputMode::Exit => None,
            InputMode::Question { .. } => Some(Self::question_help()),
        };

        let help_items = active_help.unwrap_or(vec![("Ctrl+C", "quit"), ("Ctrl+D", "cancel")]);
        let help_para = Paragraph::new(self.help_spec_to_text(help_items.clone()))
            .block(Block::default().borders(Borders::NONE));
        frame.render_widget(help_para, chunks[2]);
    }

    pub fn wrap_line(line: &Line<'static>, width: u16) -> Vec<Line<'static>> {
        let width = width.max(1) as usize;
        let line_width = line.width();
        if line_width <= width {
            return vec![line.clone()];
        }

        let mut wrapped = Vec::new();
        let mut current_spans = Vec::new();
        let mut current_width = 0usize;

        for span in &line.spans {
            let mut content = String::new();
            for ch in span.content.chars() {
                if current_width == width {
                    wrapped.push(Self::line_from_spans(
                        line,
                        std::mem::take(&mut current_spans),
                    ));
                    current_width = 0;
                }

                content.push(ch);
                current_width += 1;

                if current_width == width {
                    current_spans.push(Span::styled(std::mem::take(&mut content), span.style));
                    wrapped.push(Self::line_from_spans(
                        line,
                        std::mem::take(&mut current_spans),
                    ));
                    current_width = 0;
                }
            }

            if !content.is_empty() {
                current_spans.push(Span::styled(content, span.style));
            }
        }

        if !current_spans.is_empty() || wrapped.is_empty() {
            wrapped.push(Self::line_from_spans(line, current_spans));
        }

        wrapped
    }

    fn line_from_spans(source: &Line<'static>, spans: Vec<Span<'static>>) -> Line<'static> {
        let mut line = Line::from(spans);
        line.style = source.style;
        line.alignment = source.alignment;
        line
    }

    fn help_spec_to_text(&self, spec: Vec<(&str, &str)>) -> Text<'static> {
        let mut spans = Vec::new();
        let style = Style::default().fg(self.theme.header);
        let dim = Style::default().dim();
        spans.push(Span::raw("  "));
        for (i, (label, desc)) in spec.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(label.to_string(), style));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(desc.to_string(), dim));
        }
        Text::from(Line::from(spans))
    }
}
