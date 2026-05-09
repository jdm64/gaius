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
    agent::{AgentEvent, LLMAgent},
    config::Config,
    models::{AvailableModel, Models},
    session::Session,
};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use genai::chat::{ChatRequest, ChatRole, ContentPart};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListDirection, ListItem, Padding, Paragraph, Wrap},
};
use std::{
    error::Error,
    io::{self, Stdout},
    mem,
    time::Duration,
};

const INPUT_HEIGHT: u16 = 3;

pub enum InputMode {
    PromptInput,
    Command {
        selected: usize,
        filtered: Vec<Command>,
    },
    Session {
        selected: usize,
        sessions: Vec<String>,
    },
    Models {
        selected: usize,
        models: Vec<AvailableModel>,
    },
}

pub struct TuiApp {
    model: String,
    input: String,
    input_cursor: usize,
    messages: Vec<TuiMessage>,
    status: String,
    mode: InputMode,
}

#[derive(Clone)]
pub struct Command {
    name: &'static str,
    description: &'static str,
}

struct TuiMessage {
    role: MessageRole,
    text: String,
}

#[derive(Debug, PartialEq)]
enum MessageRole {
    User,
    Agent,
    ToolCall,
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TuiApp {
    pub fn new() -> Self {
        Self {
            model: String::new(),
            input: String::new(),
            input_cursor: 0,
            messages: Vec::new(),
            status: "Esc or Ctrl-C to quit".to_string(),
            mode: InputMode::PromptInput,
        }
    }

    fn commands() -> Vec<Command> {
        vec![
            Command {
                name: "new",
                description: "Clear history and create a new session",
            },
            Command {
                name: "sessions",
                description: "Load and delete sessions",
            },
            Command {
                name: "models",
                description: "List and select models",
            },
        ]
    }

    fn command_mode_for_input(&self) -> Option<InputMode> {
        let input = self.input.as_str();
        let query = input.strip_prefix('/')?.to_lowercase();
        let filtered: Vec<Command> = Self::commands()
            .into_iter()
            .filter(|cmd| cmd.name.to_lowercase().contains(&query))
            .collect();

        if filtered.is_empty() {
            None
        } else {
            Some(InputMode::Command {
                selected: 0,
                filtered,
            })
        }
    }

    fn mode_for_input(&self) -> InputMode {
        self.command_mode_for_input()
            .unwrap_or(InputMode::PromptInput)
    }

    fn input_len(&self) -> usize {
        self.input.chars().count()
    }

    fn input_cursor_byte_index(&self) -> usize {
        if self.input_cursor == self.input_len() {
            return self.input.len();
        }

        self.input
            .char_indices()
            .nth(self.input_cursor)
            .map(|(index, _)| index)
            .unwrap_or(self.input.len())
    }

    fn clear_input(&mut self) {
        self.input.clear();
        self.input_cursor = 0;
    }

    fn insert_input_char(&mut self, ch: char) {
        let index = self.input_cursor_byte_index();
        self.input.insert(index, ch);
        self.input_cursor += 1;
    }

    fn delete_input_char_before_cursor(&mut self) {
        if self.input_cursor == 0 {
            return;
        }

        self.input_cursor -= 1;
        let index = self.input_cursor_byte_index();
        self.input.remove(index);
    }

    fn delete_input_char_at_cursor(&mut self) {
        if self.input_cursor == self.input_len() {
            return;
        }

        let index = self.input_cursor_byte_index();
        self.input.remove(index);
    }

    fn move_input_cursor_left(&mut self) {
        self.input_cursor = self.input_cursor.saturating_sub(1);
    }

    fn move_input_cursor_right(&mut self) {
        self.input_cursor = (self.input_cursor + 1).min(self.input_len());
    }

    fn move_input_cursor_home(&mut self) {
        self.input_cursor = 0;
    }

    fn move_input_cursor_end(&mut self) {
        self.input_cursor = self.input_len();
    }

    fn filtered_model_indices(&self, models: &[AvailableModel]) -> Vec<usize> {
        let query = self.input.trim().to_lowercase();
        models
            .iter()
            .enumerate()
            .filter_map(|(index, model)| {
                let is_match = query.is_empty() || model.id.to_lowercase().contains(&query);
                is_match.then_some(index)
            })
            .collect()
    }

    fn clamp_model_selection(&self, selected: usize, models: &[AvailableModel]) -> usize {
        let filtered_len = self.filtered_model_indices(models).len();
        selected.min(filtered_len.saturating_sub(1))
    }

    async fn execute_command(
        &mut self,
        agent: &mut LLMAgent,
        config: &Config,
        command: &str,
    ) -> InputMode {
        match command {
            "new" => {
                match agent.new_session() {
                    Ok(_) => {
                        self.messages.clear();
                        self.status = "New session created".to_string();
                    }
                    Err(e) => {
                        self.status = e.to_string();
                    }
                };
                self.clear_input();
                InputMode::PromptInput
            }
            "sessions" => {
                let sessions = Session::list();
                self.clear_input();
                InputMode::Session {
                    selected: 0,
                    sessions,
                }
            }
            "models" => {
                self.clear_input();
                self.status = "Loading models...".to_string();
                match Models::list(config).await {
                    Ok(models) => {
                        self.status = format!("Loaded {} models", models.len());
                        InputMode::Models {
                            selected: 0,
                            models,
                        }
                    }
                    Err(err) => {
                        self.status = format!("Error loading models: {}", err);
                        InputMode::PromptInput
                    }
                }
            }
            _ => {
                self.messages.push(TuiMessage {
                    role: MessageRole::Agent,
                    text: format!("Unknown command: /{}", command),
                });
                self.clear_input();
                InputMode::PromptInput
            }
        }
    }

    pub async fn run(
        &mut self,
        agent: &mut LLMAgent,
        config: &Config,
    ) -> Result<(), Box<dyn Error>> {
        self.model = agent.model().clone();
        self.load_history(agent.history());

        let mut guard = TerminalGuard::enter()?;

        loop {
            guard.terminal.draw(|frame| self.draw(frame))?;

            if !event::poll(Duration::from_millis(100))? {
                continue;
            }

            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            let mode = mem::replace(&mut self.mode, InputMode::PromptInput);
            self.mode = match mode {
                InputMode::PromptInput => {
                    self.handle_prompt_input(key, &mut guard, agent, config)
                        .await?
                }
                InputMode::Command { selected, filtered } => {
                    self.handle_command_mode(key, selected, filtered, agent, config)
                        .await
                }
                InputMode::Session { selected, sessions } => {
                    self.handle_session_mode(key, selected, sessions, agent)
                }
                InputMode::Models { selected, models } => {
                    self.handle_models_mode(key, selected, models, agent, config)
                        .await
                }
            };
        }
    }

    async fn handle_prompt_input(
        &mut self,
        key: event::KeyEvent,
        guard: &mut TerminalGuard,
        agent: &mut LLMAgent,
        config: &Config,
    ) -> Result<InputMode, Box<dyn Error>> {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Err("Ctrl-C pressed".into());
            }
            KeyCode::Esc => return Err("Esc pressed".into()),
            KeyCode::Backspace => {
                self.delete_input_char_before_cursor();
                return Ok(self.mode_for_input());
            }
            KeyCode::Delete => {
                self.delete_input_char_at_cursor();
                return Ok(self.mode_for_input());
            }
            KeyCode::Left => {
                self.move_input_cursor_left();
                return Ok(InputMode::PromptInput);
            }
            KeyCode::Right => {
                self.move_input_cursor_right();
                return Ok(InputMode::PromptInput);
            }
            KeyCode::Home => {
                self.move_input_cursor_home();
                return Ok(InputMode::PromptInput);
            }
            KeyCode::End => {
                self.move_input_cursor_end();
                return Ok(InputMode::PromptInput);
            }
            KeyCode::Enter => {
                let prompt = self.input.trim().to_string();
                if prompt.is_empty() {
                    return Ok(InputMode::PromptInput);
                }

                if let Some(command) = prompt.strip_prefix('/') {
                    return Ok(self.execute_command(agent, config, command).await);
                }

                self.clear_input();
                self.messages.push(TuiMessage {
                    role: MessageRole::User,
                    text: prompt.clone(),
                });
                self.status = "Waiting for agent...".to_string();
                guard.terminal.draw(|frame| self.draw(frame))?;

                let mut events = Vec::new();
                let result = agent
                    .run_turn_with_events(prompt, |event| events.push(event))
                    .await;

                match result {
                    Ok(()) => {
                        for event in events {
                            match event {
                                AgentEvent::AgentMessage(text) => {
                                    self.messages.push(TuiMessage {
                                        role: MessageRole::Agent,
                                        text,
                                    });
                                }
                                AgentEvent::ToolCall { name, arguments } => {
                                    self.messages.push(TuiMessage {
                                        role: MessageRole::ToolCall,
                                        text: format!("{} ({})", name, arguments),
                                    });
                                }
                            }
                        }
                        self.status = "Esc or Ctrl-C to quit".to_string();
                    }
                    Err(err) => {
                        self.messages.push(TuiMessage {
                            role: MessageRole::Agent,
                            text: format!("Error: {}", err),
                        });
                        self.status = "Agent request failed".to_string();
                    }
                };
                Ok(InputMode::PromptInput)
            }
            KeyCode::Char(ch) => {
                self.insert_input_char(ch);
                return Ok(self.mode_for_input());
            }
            _ => Ok(InputMode::PromptInput),
        }
    }

    async fn handle_command_mode(
        &mut self,
        key: event::KeyEvent,
        mut selected: usize,
        filtered: Vec<Command>,
        agent: &mut LLMAgent,
        config: &Config,
    ) -> InputMode {
        match key.code {
            KeyCode::Esc => {
                self.clear_input();
                InputMode::PromptInput
            }
            KeyCode::Up if selected > 0 => {
                selected -= 1;
                InputMode::Command { selected, filtered }
            }
            KeyCode::Down if selected + 1 < filtered.len() => {
                selected += 1;
                InputMode::Command { selected, filtered }
            }
            KeyCode::Enter if !filtered.is_empty() => {
                let command = filtered[selected].name;
                self.execute_command(agent, config, command).await
            }
            KeyCode::Backspace => {
                self.delete_input_char_before_cursor();
                self.mode_for_input()
            }
            KeyCode::Delete => {
                self.delete_input_char_at_cursor();
                self.mode_for_input()
            }
            KeyCode::Left => {
                self.move_input_cursor_left();
                InputMode::Command { selected, filtered }
            }
            KeyCode::Right => {
                self.move_input_cursor_right();
                InputMode::Command { selected, filtered }
            }
            KeyCode::Home => {
                self.move_input_cursor_home();
                InputMode::Command { selected, filtered }
            }
            KeyCode::End => {
                self.move_input_cursor_end();
                InputMode::Command { selected, filtered }
            }
            KeyCode::Char(ch) => {
                self.insert_input_char(ch);
                self.mode_for_input()
            }
            _ => InputMode::Command { selected, filtered },
        }
    }

    fn handle_session_mode(
        &mut self,
        key: event::KeyEvent,
        mut selected: usize,
        mut sessions: Vec<String>,
        agent: &mut LLMAgent,
    ) -> InputMode {
        match key.code {
            KeyCode::Esc => InputMode::PromptInput,
            KeyCode::Up if selected > 0 => {
                selected -= 1;
                InputMode::Session { selected, sessions }
            }
            KeyCode::Down if selected + 1 < sessions.len() => {
                selected += 1;
                InputMode::Session { selected, sessions }
            }
            KeyCode::Enter if !sessions.is_empty() => {
                let session_id = &sessions[selected];
                match agent.load_session_by_id(session_id) {
                    Ok(()) => {
                        self.messages.clear();
                        self.status = format!("Loaded session: {}", session_id);
                        self.load_history(agent.history());
                        InputMode::PromptInput
                    }
                    Err(e) => {
                        self.status = format!("Error loading session: {}", e);
                        InputMode::Session { selected, sessions }
                    }
                }
            }
            KeyCode::Char('d')
                if key.modifiers.contains(KeyModifiers::CONTROL) && !sessions.is_empty() =>
            {
                let session_id = sessions[selected].clone();
                if let Err(e) = Session::delete(&session_id) {
                    self.status = format!("Error deleting session: {}", e);
                } else {
                    sessions = Session::list();
                    if selected >= sessions.len() && selected > 0 {
                        selected -= 1;
                    }
                    self.status = format!("Deleted session: {}", session_id);
                }
                InputMode::Session { selected, sessions }
            }
            _ => InputMode::Session { selected, sessions },
        }
    }

    async fn handle_models_mode(
        &mut self,
        key: event::KeyEvent,
        mut selected: usize,
        models: Vec<AvailableModel>,
        agent: &mut LLMAgent,
        config: &Config,
    ) -> InputMode {
        match key.code {
            KeyCode::Esc => {
                self.clear_input();
                InputMode::PromptInput
            }
            KeyCode::Up if selected > 0 => {
                selected -= 1;
                InputMode::Models { selected, models }
            }
            KeyCode::Down if selected + 1 < self.filtered_model_indices(&models).len() => {
                selected += 1;
                InputMode::Models { selected, models }
            }
            KeyCode::Enter => {
                let filtered = self.filtered_model_indices(&models);
                if filtered.is_empty() {
                    self.status = "No matching models".to_string();
                    return InputMode::Models { selected, models };
                }

                selected = selected.min(filtered.len().saturating_sub(1));
                let selected_model = &models[filtered[selected]];
                match selected_model.create_client(config) {
                    Ok(client) => {
                        let model_id = selected_model.id.clone();
                        agent.set_model(client, model_id.clone());
                        self.model = model_id.clone();
                        self.clear_input();
                        self.status = format!("Selected model: {}", model_id);
                        InputMode::PromptInput
                    }
                    Err(err) => {
                        self.status = format!("Error selecting model: {}", err);
                        InputMode::Models { selected, models }
                    }
                }
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.status = "Reloading models...".to_string();
                match Models::reload(config).await {
                    Ok(models) => {
                        let selected = self.clamp_model_selection(selected, &models);
                        self.status = format!("Reloaded {} models", models.len());
                        InputMode::Models { selected, models }
                    }
                    Err(err) => {
                        self.status = format!("Error reloading models: {}", err);
                        InputMode::Models { selected, models }
                    }
                }
            }
            KeyCode::Backspace => {
                self.delete_input_char_before_cursor();
                selected = self.clamp_model_selection(selected, &models);
                InputMode::Models { selected, models }
            }
            KeyCode::Delete => {
                self.delete_input_char_at_cursor();
                selected = self.clamp_model_selection(selected, &models);
                InputMode::Models { selected, models }
            }
            KeyCode::Left => {
                self.move_input_cursor_left();
                InputMode::Models { selected, models }
            }
            KeyCode::Right => {
                self.move_input_cursor_right();
                InputMode::Models { selected, models }
            }
            KeyCode::Home => {
                self.move_input_cursor_home();
                InputMode::Models { selected, models }
            }
            KeyCode::End => {
                self.move_input_cursor_end();
                InputMode::Models { selected, models }
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.insert_input_char(ch);
                selected = self.clamp_model_selection(selected, &models);
                InputMode::Models { selected, models }
            }
            _ => InputMode::Models { selected, models },
        }
    }

    fn load_history(&mut self, history: &ChatRequest) {
        if let Some(system) = &history.system
            && !system.trim().is_empty()
        {
            self.messages.push(TuiMessage {
                role: MessageRole::Agent,
                text: format!("system: {}", system),
            });
        }

        for message in &history.messages {
            for tui_message in tui_messages_for_chat_message(message.role.clone(), &message.content)
            {
                self.messages.push(tui_message);
            }
        }
    }

    fn draw(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(INPUT_HEIGHT)])
            .split(area);

        frame.render_widget(Clear, area);
        self.draw_history(frame, chunks[0]);
        self.draw_input(frame, chunks[1]);

        match &self.mode {
            InputMode::Command { selected, filtered } => {
                self.draw_commands(frame, chunks[1], *selected, filtered);
            }
            InputMode::Session { selected, sessions } => {
                self.draw_sessions(frame, chunks[1], *selected, sessions);
            }
            InputMode::Models { selected, models } => {
                self.draw_models(frame, chunks[1], *selected, models);
            }
            InputMode::PromptInput => {}
        }
    }

    fn draw_commands(
        &self,
        frame: &mut Frame<'_>,
        input_area: Rect,
        selected: usize,
        filtered: &[Command],
    ) {
        let command_count = filtered.len() as u16;
        let visible_commands = command_count.min(10);
        let width = 50.min(input_area.width - 4);
        let height = visible_commands + 2;
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

        let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Commands"));

        frame.render_widget(Clear, rect);
        frame.render_widget(list, rect);
    }

    fn draw_sessions(
        &self,
        frame: &mut Frame<'_>,
        input_area: Rect,
        selected: usize,
        sessions: &[String],
    ) {
        let session_count = sessions.len() as u16;
        let visible_sessions = session_count.min(10);
        let help_height = 3u16;
        let width = 50.min(input_area.width - 4);
        let height = visible_sessions + 2 + help_height;
        let x = input_area.x + 2;
        let y = input_area.y - height;
        let rect = Rect::new(x, y, width, height);

        let items: Vec<ListItem> = sessions
            .iter()
            .enumerate()
            .map(|(i, session_id)| {
                if i == selected {
                    ListItem::new(session_id.as_str()).style(Style::default().bg(Color::DarkGray))
                } else {
                    ListItem::new(session_id.as_str())
                }
            })
            .collect();

        let sessions_list =
            List::new(items).block(Block::default().borders(Borders::ALL).title("Sessions"));

        let help_text = "Enter: load | Ctrl+D: delete | Esc: close";
        let help_para = Paragraph::new(help_text)
            .block(Block::default().borders(Borders::ALL))
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
        &self,
        frame: &mut Frame<'_>,
        input_area: Rect,
        selected: usize,
        models: &[AvailableModel],
    ) {
        let filtered = self.filtered_model_indices(models);
        let result_count = filtered.len();
        let selected = selected.min(result_count.saturating_sub(1));
        let visible_models = (result_count as u16).clamp(1, 10);
        let help_height = 3u16;
        let width = 70.min(input_area.width - 4);
        let height = visible_models + 2 + help_height;
        let x = input_area.x + 2;
        let y = input_area.y - height;
        let rect = Rect::new(x, y, width, height);

        let start = if selected >= visible_models as usize {
            selected + 1 - visible_models as usize
        } else {
            0
        };
        let end = (start + visible_models as usize).min(result_count);

        let items: Vec<ListItem> = if result_count == 0 {
            vec![ListItem::new("No matching models")]
        } else {
            filtered[start..end]
                .iter()
                .enumerate()
                .map(|(offset, model_index)| {
                    let i = start + offset;
                    let model = &models[*model_index];
                    let label = model.label();
                    if i == selected {
                        ListItem::new(label).style(Style::default().bg(Color::DarkGray))
                    } else {
                        ListItem::new(label)
                    }
                })
                .collect()
        };

        let models_list =
            List::new(items).block(Block::default().borders(Borders::ALL).title("Models"));

        let help_text = "Type: filter | Enter: select | Ctrl+R: reload | Esc: close";
        let help_para = Paragraph::new(help_text)
            .block(Block::default().borders(Borders::ALL))
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

    fn draw_history(&self, frame: &mut Frame<'_>, area: Rect) {
        let mut items = Vec::new();
        for (index, message) in self.messages.iter().rev().enumerate() {
            if index > 0 {
                let previous = &self.messages[self.messages.len() - index];
                if previous.role != message.role {
                    items.push(ListItem::new(""));
                }
            }

            let (label, style) = message.role.parts();
            let lines = message
                .text
                .lines()
                .enumerate()
                .map(|(index, line)| {
                    if index == 0 {
                        Line::from(vec![
                            Span::styled(
                                format!("{} ", label),
                                Style::default().add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(line.to_string()),
                        ])
                    } else {
                        Line::from(format!("  {}", line))
                    }
                })
                .collect::<Vec<_>>();

            items.push(ListItem::new(lines).style(style));
        }

        let history = List::new(items)
            .direction(ListDirection::BottomToTop)
            .block(
                Block::default()
                    .title(format!(" Gaius - {} ", self.model))
                    .borders(Borders::ALL)
                    .padding(Padding::horizontal(1)),
            )
            .highlight_style(Style::default());
        frame.render_widget(history, area);
    }

    fn draw_input(&self, frame: &mut Frame<'_>, area: Rect) {
        let input = Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Green)),
            Span::raw(self.input.as_str()),
        ]))
        .block(
            Block::default()
                .title(format!(" {} ", self.status))
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1)),
        )
        .wrap(Wrap { trim: false });
        frame.render_widget(input, area);

        let cursor_x = area.x + 4 + self.input_cursor as u16;
        let cursor_y = area.y + 1;
        if cursor_x < area.x + area.width {
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }
}

fn tui_messages_for_chat_message(
    role: ChatRole,
    content: &genai::chat::MessageContent,
) -> Vec<TuiMessage> {
    let mut messages = Vec::new();

    for part in content {
        match part {
            ContentPart::Text(text) if !text.is_empty() => {
                messages.push(TuiMessage {
                    role: message_role_for_chat_role(&role),
                    text: text.clone(),
                });
            }
            ContentPart::ToolCall(tool_call) => {
                messages.push(TuiMessage {
                    role: MessageRole::ToolCall,
                    text: format!("{} ({})", tool_call.fn_name, tool_call.fn_arguments),
                });
            }
            ContentPart::ToolResponse(tool_response) => {
                messages.push(TuiMessage {
                    role: MessageRole::ToolCall,
                    text: format!("{} => {}", tool_response.call_id, tool_response.content),
                });
            }
            ContentPart::Text(_) | ContentPart::Binary(_) | ContentPart::ThoughtSignature(_) => {}
        }
    }

    messages
}

fn message_role_for_chat_role(role: &ChatRole) -> MessageRole {
    match role {
        ChatRole::User => MessageRole::User,
        ChatRole::Assistant | ChatRole::System => MessageRole::Agent,
        ChatRole::Tool => MessageRole::ToolCall,
    }
}

impl MessageRole {
    fn parts(&self) -> (&'static str, Style) {
        match self {
            Self::User => (
                "user>",
                Style::default().fg(Color::White).bg(Color::DarkGray),
            ),
            Self::Agent => ("agent>", Style::default()),
            Self::ToolCall => (
                "tool-call>",
                Style::default().fg(Color::Cyan).bg(Color::Black),
            ),
        }
    }
}

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genai::chat::{ChatMessage, MessageContent, ToolCall, ToolResponse};

    #[test]
    fn edits_input_at_cursor() {
        let mut app = TuiApp::new();
        app.insert_input_char('a');
        app.insert_input_char('c');

        app.move_input_cursor_left();
        app.insert_input_char('b');

        assert_eq!(app.input, "abc");
        assert_eq!(app.input_cursor, 2);

        app.delete_input_char_before_cursor();

        assert_eq!(app.input, "ac");
        assert_eq!(app.input_cursor, 1);

        app.delete_input_char_at_cursor();

        assert_eq!(app.input, "a");
        assert_eq!(app.input_cursor, 1);
    }

    #[test]
    fn moves_input_cursor_home_and_end() {
        let mut app = TuiApp::new();
        for ch in "prompt".chars() {
            app.insert_input_char(ch);
        }

        app.move_input_cursor_home();
        assert_eq!(app.input_cursor, 0);

        app.move_input_cursor_end();
        assert_eq!(app.input_cursor, 6);
    }

    #[test]
    fn edits_multibyte_input_at_cursor() {
        let mut app = TuiApp::new();
        for ch in "aéc".chars() {
            app.insert_input_char(ch);
        }

        app.move_input_cursor_left();
        app.insert_input_char('b');
        app.move_input_cursor_left();
        app.delete_input_char_before_cursor();

        assert_eq!(app.input, "abc");
        assert_eq!(app.input_cursor, 1);
    }

    #[test]
    fn loads_chat_request_into_tui_history() {
        let request = ChatRequest {
            system: Some("be useful".to_string()),
            messages: vec![
                ChatMessage::user("hello"),
                ChatMessage::assistant(MessageContent::from_parts(vec![
                    ContentPart::Text("hi".to_string()),
                    ContentPart::ToolCall(ToolCall {
                        call_id: "123".to_string(),
                        fn_name: "weather".to_string(),
                        fn_arguments: serde_json::json!({"city": "Atlanta"}),
                        thought_signatures: None,
                    }),
                ])),
                ChatMessage {
                    role: ChatRole::Tool,
                    content: MessageContent::from_parts(vec![ContentPart::ToolResponse(
                        ToolResponse::new("123", "sunny"),
                    )]),
                    options: None,
                },
            ],
            tools: None,
        };

        let mut app = TuiApp::new();
        app.load_history(&request);

        assert_eq!(app.messages.len(), 5);
        assert_eq!(app.messages[0].role, MessageRole::Agent);
        assert_eq!(app.messages[0].text, "system: be useful");
        assert_eq!(app.messages[1].role, MessageRole::User);
        assert_eq!(app.messages[1].text, "hello");
        assert_eq!(app.messages[2].role, MessageRole::Agent);
        assert_eq!(app.messages[2].text, "hi");
        assert_eq!(app.messages[3].role, MessageRole::ToolCall);
        assert_eq!(app.messages[3].text, "weather ({\"city\":\"Atlanta\"})");
        assert_eq!(app.messages[4].role, MessageRole::ToolCall);
        assert_eq!(app.messages[4].text, "123 => sunny");
    }
}
