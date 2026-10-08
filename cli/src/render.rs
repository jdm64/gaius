/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

pub mod history;
pub mod input;
pub mod layout;
pub mod lists;
pub mod util;

use crate::{input::InputMode, theme::ColorTheme, tui::TuiApp};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders, Clear, Paragraph},
};

pub const INPUT_HEIGHT: u16 = 3;

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

    pub fn draw(&mut self, app: &mut TuiApp, frame: &mut Frame<'_>) {
        self.theme = app.display_prefs.theme.clone();

        let area = frame.area();
        let input_width = area.width.saturating_sub(4).max(1);

        let question_lines = self.question_lines(&app.editor.mode, input_width);
        let input_prompt = self.input_prompt_lines(app.editor.input.clone(), input_width);
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

        let active_help: Option<Vec<(&'static str, &'static str)>> = match &app.editor.mode {
            InputMode::Command { picker } => self.draw_commands(frame, chunks[1], picker),
            InputMode::Session { picker } => self.draw_sessions(frame, chunks[1], picker, false),
            InputMode::SessionRename { picker } => {
                self.draw_sessions(frame, chunks[1], picker, true)
            }
            InputMode::Models { picker } => self.draw_models(frame, chunks[1], picker),
            InputMode::AddProvider { picker } => {
                self.draw_add_provider(frame, chunks[1], picker, app.editor.input.as_str())
            }
            InputMode::Agents { picker } => self.draw_agents(frame, chunks[1], picker),
            InputMode::Files { picker } => self.draw_files(frame, chunks[1], picker),
            InputMode::Skills { picker } => self.draw_skills(frame, chunks[1], picker),
            InputMode::PromptInput | InputMode::Exit => None,
            InputMode::SessionInfo { info } => self.draw_session_info(frame, chunks[1], info),
            InputMode::Question { .. } => Some(Self::question_help()),
            InputMode::Reasoning { picker } => self.draw_reasoning(frame, chunks[1], picker),
            InputMode::Theme { picker } => self.draw_theme(frame, chunks[1], picker),
        };

        let help_items = active_help.unwrap_or(vec![("Ctrl+C", "quit"), ("Ctrl+D", "cancel")]);
        let help_para = Paragraph::new(self.theme.help_spec_to_text(help_items.clone()))
            .block(Block::default().borders(Borders::NONE));
        frame.render_widget(help_para, chunks[2]);
    }
}
