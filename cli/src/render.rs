/* Copyright 2026 Justin Madru (justin.jdm64@gmail.com)
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use crate::{
    agents::AgentDefinition,
    commands::{Command, Commands},
    input::InputMode,
    models::{AvailableModel, ModelPickerRow, model_picker_rows},
    session::Session,
    tui::{MessageRole, TuiApp, TuiMessage, wrapped_line_count},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap},
};
use tui_markdown::{Options, from_str_with_options};

pub const INPUT_HEIGHT: u16 = 3;

pub struct Render {}

impl Render {
    pub fn draw(app: &mut TuiApp, frame: &mut Frame<'_>) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(INPUT_HEIGHT)])
            .split(area);

        frame.render_widget(Clear, area);
        Self::draw_history(app, frame, chunks[0]);
        Self::draw_input(app, frame, chunks[1]);

        match &app.mode {
            InputMode::Command { selected, filtered } => {
                Self::draw_commands(app, frame, chunks[1], *selected, filtered);
            }
            InputMode::Session { selected, sessions } => {
                Self::draw_sessions(app, frame, chunks[1], *selected, sessions, false);
            }
            InputMode::SessionRename { selected, sessions } => {
                Self::draw_sessions(app, frame, chunks[1], *selected, sessions, true);
            }
            InputMode::Models {
                selected,
                models,
                recent_models,
            } => {
                Self::draw_models(app, frame, chunks[1], *selected, models, recent_models);
            }
            InputMode::Agents { selected, agents } => {
                Self::draw_agents(app, frame, chunks[1], *selected, agents);
            }
            InputMode::PromptInput | InputMode::Exit => {}
        }
    }

    fn draw_commands(
        _app: &TuiApp,
        frame: &mut Frame<'_>,
        input_area: Rect,
        selected: usize,
        filtered: &[Command],
    ) {
        let command_count = filtered.len() as u16;
        let visible_commands = command_count.min(10);
        let help_height = 1u16;
        let width = 50.min(input_area.width - 4);
        let height = visible_commands + 2 + help_height;
        let x = input_area.x + 2;
        let y = input_area.y - height;
        let rect = Rect::new(x, y, width, height);

        let items: Vec<ListItem> = filtered
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let content = format!("/{} - {}", cmd.name, cmd.description);
                if i == selected {
                    ListItem::new(content).style(Style::default().bg(Color::DarkGray))
                } else {
                    ListItem::new(content)
                }
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Commands")
                .padding(Padding::horizontal(1)),
        );

        let help_text = "  Type: filter | Enter: select | Esc: close";
        let help_para = Paragraph::new(help_text)
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().fg(Color::Yellow));

        frame.render_widget(Clear, rect);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(visible_commands + 2),
                Constraint::Length(help_height),
            ])
            .split(rect);

        frame.render_widget(list, chunks[0]);
        frame.render_widget(help_para, chunks[1]);
    }

    fn draw_sessions(
        _app: &TuiApp,
        frame: &mut Frame<'_>,
        input_area: Rect,
        selected: usize,
        sessions: &[Session],
        renaming: bool,
    ) {
        let session_count = sessions.len();
        let selected = selected.min(session_count.saturating_sub(1));
        let visible_sessions = (session_count as u16).clamp(1, 10);
        let help_height = 1u16;
        let width = 50.min(input_area.width - 4);
        let height = visible_sessions + 2 + help_height;
        let x = input_area.x + 2;
        let y = input_area.y - height;
        let rect = Rect::new(x, y, width, height);

        let start = if selected >= visible_sessions as usize {
            selected + 1 - visible_sessions as usize
        } else {
            0
        };
        let end = (start + visible_sessions as usize).min(session_count);

        let items: Vec<ListItem> = if session_count == 0 {
            vec![ListItem::new("No sessions")]
        } else {
            sessions[start..end]
                .iter()
                .enumerate()
                .map(|(offset, session)| {
                    let i = start + offset;
                    let label = session.display_name();
                    if i == selected {
                        ListItem::new(label).style(Style::default().bg(Color::DarkGray))
                    } else {
                        ListItem::new(label)
                    }
                })
                .collect()
        };

        let sessions_list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Sessions")
                .padding(Padding::horizontal(1)),
        );

        let help_text = if renaming {
            "  Enter: save | Esc: cancel"
        } else {
            "  Enter: load | Ctrl+E: rename | Ctrl+D: delete | Esc: close"
        };
        let help_para = Paragraph::new(help_text)
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().fg(Color::Yellow));

        frame.render_widget(Clear, rect);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(visible_sessions + 2),
                Constraint::Length(help_height),
            ])
            .split(rect);

        frame.render_widget(sessions_list, chunks[0]);
        frame.render_widget(help_para, chunks[1]);
    }

    fn draw_models(
        app: &TuiApp,
        frame: &mut Frame<'_>,
        input_area: Rect,
        selected: usize,
        models: &[AvailableModel],
        recent_models: &[AvailableModel],
    ) {
        let rows = model_picker_rows(&app.input, models, recent_models);
        let result_count = rows.len();
        let selected = selected.min(Self::selectable_row_count(&rows).saturating_sub(1));
        let selected_row = Self::selected_row_index(&rows, selected);
        let visible_models = (result_count as u16).clamp(1, 10);
        let help_height = 1u16;
        let width = 70.min(input_area.width - 4);
        let height = visible_models + 2 + help_height;
        let x = input_area.x + 2;
        let y = input_area.y - height;
        let rect = Rect::new(x, y, width, height);

        let start = if selected_row >= visible_models as usize {
            selected_row + 1 - visible_models as usize
        } else {
            0
        };
        let end = (start + visible_models as usize).min(result_count);

        let items: Vec<ListItem> = if result_count == 0 {
            vec![ListItem::new("No matching models")]
        } else {
            rows[start..end]
                .iter()
                .enumerate()
                .map(|(offset, row)| {
                    let row_index = start + offset;
                    match row {
                        ModelPickerRow::Header(label) => {
                            ListItem::new(label.as_str()).style(Style::default().fg(Color::Yellow))
                        }
                        ModelPickerRow::Separator => ListItem::new(""),
                        ModelPickerRow::Model(model) => {
                            let item = ListItem::new(model.label());
                            if row_index == selected_row {
                                item.style(Style::default().bg(Color::DarkGray))
                            } else {
                                item
                            }
                        }
                    }
                })
                .collect()
        };

        let models_list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Models")
                .padding(Padding::horizontal(1)),
        );

        let help_text = "  Type: filter | Enter: select | Ctrl+R: reload | Esc: close";
        let help_para = Paragraph::new(help_text)
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().fg(Color::Yellow));

        frame.render_widget(Clear, rect);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(visible_models + 2),
                Constraint::Length(help_height),
            ])
            .split(rect);

        frame.render_widget(models_list, chunks[0]);
        frame.render_widget(help_para, chunks[1]);
    }

    fn selectable_row_count(rows: &[ModelPickerRow]) -> usize {
        rows.iter()
            .filter(|row| matches!(row, ModelPickerRow::Model(_)))
            .count()
    }

    fn selected_row_index(rows: &[ModelPickerRow], selected: usize) -> usize {
        let mut model_index = 0;
        for (row_index, row) in rows.iter().enumerate() {
            if matches!(row, ModelPickerRow::Model(_)) {
                if model_index == selected {
                    return row_index;
                }
                model_index += 1;
            }
        }

        0
    }

    fn draw_agents(
        app: &TuiApp,
        frame: &mut Frame<'_>,
        input_area: Rect,
        selected: usize,
        agents: &[AgentDefinition],
    ) {
        let filtered = Commands::filtered_agent_indices(&app.input, agents);
        let result_count = filtered.len();
        let selected = selected.min(result_count.saturating_sub(1));
        let visible_agents = (result_count as u16).clamp(1, 10);
        let help_height = 1u16;
        let width = 60.min(input_area.width - 4);
        let height = visible_agents + 2 + help_height;
        let x = input_area.x + 2;
        let y = input_area.y - height;
        let rect = Rect::new(x, y, width, height);

        let start = if selected >= visible_agents as usize {
            selected + 1 - visible_agents as usize
        } else {
            0
        };
        let end = (start + visible_agents as usize).min(result_count);

        let items: Vec<ListItem> = if result_count == 0 {
            vec![ListItem::new("No matching agents")]
        } else {
            filtered[start..end]
                .iter()
                .enumerate()
                .map(|(offset, agent_index)| {
                    let i = start + offset;
                    let agent = &agents[*agent_index];
                    if i == selected {
                        ListItem::new(agent.name.as_str())
                            .style(Style::default().bg(Color::DarkGray))
                    } else {
                        ListItem::new(agent.name.as_str())
                    }
                })
                .collect()
        };

        let agents_list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Agents")
                .padding(Padding::horizontal(1)),
        );

        let help_text = "  Type: filter | Enter: select | Esc: close";
        let help_para = Paragraph::new(help_text)
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().fg(Color::Yellow));

        frame.render_widget(Clear, rect);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(visible_agents + 2),
                Constraint::Length(help_height),
            ])
            .split(rect);

        frame.render_widget(agents_list, chunks[0]);
        frame.render_widget(help_para, chunks[1]);
    }

    fn draw_history(app: &mut TuiApp, frame: &mut Frame<'_>, area: Rect) {
        let text_width = area.width.saturating_sub(4).max(1);
        let text_height = area.height.saturating_sub(2).max(1);
        app.history_page_size = text_height;

        let lines = Self::history_lines(&*app);
        let wrapped_height = wrapped_line_count(&lines, text_width);
        let max_scroll = wrapped_height.saturating_sub(text_height);
        let clamped_scroll = app.history_scroll.min(max_scroll);
        let scroll_offset = max_scroll.saturating_sub(clamped_scroll);

        let title = if let Some(tokens) = app.context_tokens {
            format!(
                " Gaius - {} - {} | Context: {} ",
                app.model, app.agent_name, tokens
            )
        } else {
            format!(" Gaius - {} - {} ", app.model, app.agent_name)
        };

        let history = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::TOP)
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false })
            .scroll((scroll_offset, 0));
        frame.render_widget(history, area);

        app.history_scroll = clamped_scroll;
    }

    fn history_lines(app: &TuiApp) -> Vec<Line<'_>> {
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        for (index, message) in app.messages.iter().enumerate() {
            if index > 0 {
                let previous = &app.messages[index - 1];
                if previous.role != message.role {
                    lines.push(Line::from(""));
                }
            }

            lines.extend(Self::render_message(message));
        }

        lines
    }

    pub fn render_message<'a>(msg: &'a TuiMessage) -> Vec<Line<'a>> {
        let mut lines = Vec::new();

        match msg.role {
            MessageRole::Agent => {
                let options = Options::default();
                lines.append(&mut from_str_with_options(&msg.text, &options).lines);
            }
            MessageRole::User => {
                let style = Style::default().bg(Color::Rgb(64, 64, 64)).italic().bold();
                lines.push(Line::from(msg.text.clone()).style(style));
            }
            MessageRole::ToolCall => {
                let style = Style::default().fg(Color::Cyan);
                lines.push(Line::from(msg.text.clone()).style(style));
            }
            MessageRole::System => {
                let style = Style::default().fg(Color::Rgb(64, 64, 0));
                lines.push(Line::from(msg.text.clone()).style(style));
            }
        }

        lines
    }

    fn draw_input(app: &TuiApp, frame: &mut Frame<'_>, area: Rect) {
        let input = Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Green)),
            Span::raw(app.input.as_str()),
        ]))
        .block(
            Block::default()
                .title(format!(" {} ", app.status))
                .borders(Borders::ALL)
                .style(Style::default().bg(Color::Rgb(64, 0, 64)))
                .padding(Padding::horizontal(1)),
        )
        .wrap(Wrap { trim: false });
        frame.render_widget(input, area);

        let cursor_x = area.x + 4 + app.input_cursor as u16;
        let cursor_y = area.y + 1;
        if cursor_x < area.x + area.width {
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }
}
