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

use crate::agent::{AgentEvent, LLMAgent};
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
    time::Duration,
};

const INPUT_HEIGHT: u16 = 3;

pub struct TuiApp {
    model: String,
    input: String,
    messages: Vec<TuiMessage>,
    status: String,
    show_commands: bool,
    filtered_commands: Vec<Command>,
    command_selected: usize,
}

struct Command {
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
            messages: Vec::new(),
            status: "Esc or Ctrl-C to quit".to_string(),
            show_commands: false,
            filtered_commands: Vec::new(),
            command_selected: 0,
        }
    }

    fn commands() -> Vec<Command> {
        vec![Command {
            name: "new",
            description: "Clear history and create a new session",
        }]
    }

    fn update_command_filter(&mut self) {
        let input = self.input.as_str();
        if input.starts_with('/') {
            let query = &input[1..].to_lowercase();
            self.filtered_commands = Self::commands()
                .into_iter()
                .filter(|cmd| cmd.name.to_lowercase().contains(query))
                .collect();
            self.show_commands = !self.filtered_commands.is_empty();
            self.command_selected = 0;
        } else {
            self.show_commands = false;
        }
    }

    fn execute_command(&mut self, agent: &mut LLMAgent, command: &str) {
        match command {
            "new" => {
                agent.new_session();
                self.messages.clear();
                self.status = "New session created".to_string();
            }
            _ => {
                self.messages.push(TuiMessage {
                    role: MessageRole::Agent,
                    text: format!("Unknown command: /{}", command),
                });
            }
        }
        self.show_commands = false;
        self.input.clear();
    }

    pub async fn run(&mut self, agent: &mut LLMAgent) -> Result<(), Box<dyn Error>> {
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

            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Esc => break,
                KeyCode::Backspace => {
                    self.input.pop();
                    self.update_command_filter();
                }
                KeyCode::Up => {
                    if self.show_commands && self.command_selected > 0 {
                        self.command_selected -= 1;
                    }
                }
                KeyCode::Down => {
                    if self.show_commands
                        && self.command_selected + 1 < self.filtered_commands.len()
                    {
                        self.command_selected += 1;
                    }
                }
                KeyCode::Enter => {
                    if self.show_commands && !self.filtered_commands.is_empty() {
                        let command = self.filtered_commands[self.command_selected].name;
                        self.execute_command(agent, command);
                        continue;
                    }

                    let prompt = self.input.trim().to_string();
                    if prompt.is_empty() {
                        continue;
                    }

                    if prompt.starts_with('/') {
                        let command = &prompt[1..];
                        self.execute_command(agent, command);
                        continue;
                    }

                    self.input.clear();
                    self.show_commands = false;
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
                    }
                }
                KeyCode::Char(ch) => {
                    self.input.push(ch);
                    self.update_command_filter();
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn load_history(&mut self, history: &ChatRequest) {
        if let Some(system) = &history.system {
            if !system.trim().is_empty() {
                self.messages.push(TuiMessage {
                    role: MessageRole::Agent,
                    text: format!("system: {}", system),
                });
            }
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

        if self.show_commands {
            self.draw_commands(frame, chunks[1]);
        }
    }

    fn draw_commands(&self, frame: &mut Frame<'_>, input_area: Rect) {
        let command_count = self.filtered_commands.len() as u16;
        let visible_commands = command_count.min(10);
        let width = 50.min(input_area.width - 4);
        let height = visible_commands + 2;
        let x = input_area.x + 2;
        let y = input_area.y - height;
        let rect = Rect::new(x, y, width, height);

        let items: Vec<ListItem> = self
            .filtered_commands
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let content = format!("/{} - {}", cmd.name, cmd.description);
                if i == self.command_selected {
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

        let cursor_x = area.x + 4 + self.input.chars().count() as u16;
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
