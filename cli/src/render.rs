/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{input::InputMode, render_util::USER_PROMPT_BAR, tui::TuiApp};
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

impl ColorTheme {
    pub fn format_user_prompt_line(&self, mut line: Line<'static>, width: u16) -> Line<'static> {
        let bar_style = self.user_prompt_style().fg(self.user_bar);
        line.spans
            .insert(0, Span::styled(USER_PROMPT_BAR, bar_style));
        let width = width.max(1) as usize;
        let used = line.width();
        if used < width {
            line.spans.push(Span::styled(
                " ".repeat(width - used),
                self.user_prompt_style(),
            ));
        }
        line
    }

    pub fn user_prompt_bar_line(&self) -> Line<'static> {
        let style = self.user_prompt_style();
        Line::from(vec![Span::styled(USER_PROMPT_BAR, style.fg(self.user_bar))])
    }

    pub fn user_prompt_style(&self) -> Style {
        Style::default().bg(self.user_box)
    }

    pub fn help_spec_to_text(&self, spec: Vec<(&str, &str)>) -> Text<'static> {
        let mut spans = Vec::new();
        let style = Style::default().fg(self.header);
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
            InputMode::Skills { picker } => self.draw_skills(frame, chunks[1], picker),
            InputMode::PromptInput | InputMode::Exit => None,
            InputMode::SessionInfo { info } => self.draw_session_info(frame, chunks[1], info),
            InputMode::Question { .. } => Some(Self::question_help()),
            InputMode::Reasoning { picker } => self.draw_reasoning(frame, chunks[1], picker),
        };

        let help_items = active_help.unwrap_or(vec![("Ctrl+C", "quit"), ("Ctrl+D", "cancel")]);
        let help_para = Paragraph::new(self.theme.help_spec_to_text(help_items.clone()))
            .block(Block::default().borders(Borders::NONE));
        frame.render_widget(help_para, chunks[2]);
    }
}
