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
    layout::{Constraint, Direction, Layout, Rect},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::path::Path;

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
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(input_height),
                Constraint::Length(1),
            ])
            .split(area);

        frame.render_widget(Clear, area);

        self.draw_header(app, frame, chunks[0]);
        self.draw_history(app, frame, chunks[1]);
        self.draw_input(
            app,
            frame,
            chunks[2],
            input_lines,
            input_start_line,
            input_width as usize,
        );

        let active_help = self.draw_list(app, frame, chunks[1]);
        let help_items = active_help.unwrap_or(vec![("Ctrl+C", "quit"), ("Ctrl+D", "cancel")]);
        let help_para = Paragraph::new(self.theme.help_spec_to_text(help_items.clone()))
            .block(Block::default().borders(Borders::NONE));
        frame.render_widget(help_para, chunks[3]);
    }

    fn draw_list(
        &self,
        app: &mut TuiApp,
        frame: &mut Frame<'_>,
        area: Rect,
    ) -> Option<Vec<(&'static str, &'static str)>> {
        match &app.editor.mode {
            InputMode::Command { picker } => self.draw_commands(frame, area, picker),
            InputMode::Session { picker } => {
                self.draw_sessions(frame, area, picker, app.snapshot.session_id.as_ref(), false)
            }
            InputMode::SessionRename { picker } => {
                self.draw_sessions(frame, area, picker, app.snapshot.session_id.as_ref(), true)
            }
            InputMode::Models { picker } => {
                self.draw_models(frame, area, picker, &app.snapshot.model)
            }
            InputMode::AddProvider { picker } => {
                self.draw_add_provider(frame, area, picker, app.editor.input.as_str())
            }
            InputMode::Agents { picker } => self.draw_agents(frame, area, picker),
            InputMode::Files { picker } => self.draw_files(frame, area, picker),
            InputMode::Skills { picker } => self.draw_skills(frame, area, picker),
            InputMode::PromptInput | InputMode::Exit => None,
            InputMode::SessionInfo { info } => self.draw_session_info(frame, area, info),
            InputMode::Question { .. } => Some(Self::question_help()),
            InputMode::Reasoning { picker } => {
                self.draw_reasoning(frame, area, picker, app.snapshot.model.reasoning.clone())
            }
            InputMode::Theme { picker } => self.draw_theme(frame, area, picker),
        }
    }

    fn draw_header(&self, app: &mut TuiApp, frame: &mut Frame<'_>, area: Rect) {
        let snapshot = &app.snapshot;
        let agent_label = if snapshot.plan_mode_on {
            format!("{}/plan", snapshot.agent_name)
        } else {
            snapshot.agent_name.clone()
        };
        let mut model_name = match &snapshot.model.reasoning {
            Some(reasoning) => format!("{}:{}", snapshot.model.id, reasoning),
            None => snapshot.model.id.clone(),
        };
        if model_name.is_empty() {
            model_name = "[no model]".to_string();
        }
        let parts: Vec<String> = [
            Some(format!("Gaius - {} - {}", model_name, agent_label)),
            app.context_tokens
                .map(|tokens| match snapshot.model.context_len {
                    Some(context_len) if context_len > 0 => {
                        let pct = tokens as f64 / context_len as f64 * 100.0;
                        format!(" - {} {:.0}%", tokens, pct)
                    }
                    _ => format!(" - {}", tokens),
                }),
            snapshot.total_cost.map(|cost| format!(" ${:.3}", cost)),
        ]
        .into_iter()
        .flatten()
        .collect();

        let proj_dir = tilde_path(&snapshot.proj_dir);
        let header = Paragraph::new(vec![
            Line::from(parts.join("")),
            Line::from(format!("Proj: {}", proj_dir)),
        ]);
        frame.render_widget(header, area);
    }
}

fn tilde_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", rest.display())
        };
    }
    path.display().to_string()
}
