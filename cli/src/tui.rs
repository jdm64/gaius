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
    commands::Commands,
    config::Config,
    harness::{Harness, HarnessEvent},
    input::{Input, InputMode},
    render::Render,
};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use genai::chat::{ChatRequest, ChatRole, ContentPart};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    style::{Color, Style},
    text::Line,
};
use std::{
    error::Error,
    io::{self, Stdout},
    time::Duration,
};

#[derive(Clone)]
pub struct TuiMessage {
    pub role: MessageRole,
    pub text: String,
    pub is_markdown: bool,
}

#[derive(Debug, PartialEq, Clone)]
pub enum MessageRole {
    User,
    Agent,
    ToolCall,
}

pub struct TerminalGuard {
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

pub struct TuiApp {
    pub model: String,
    pub agent_name: String,
    pub input: String,
    pub input_cursor: usize,
    pub history_scroll: u16,
    pub history_page_size: u16,
    pub messages: Vec<TuiMessage>,
    pub status: String,
    pub mode: InputMode,
    pub context_tokens: Option<i32>,
}

impl TuiApp {
    pub fn new() -> Self {
        Self {
            model: String::new(),
            agent_name: String::new(),
            input: String::new(),
            input_cursor: 0,
            history_scroll: 0,
            history_page_size: 1,
            messages: Vec::new(),
            status: "Ctrl-C to quit".to_string(),
            mode: InputMode::PromptInput,
            context_tokens: None,
        }
    }

    pub async fn run(
        &mut self,
        harness: &mut Harness,
        config: &Config,
    ) -> Result<(), Box<dyn Error>> {
        self.model = harness.model().clone();
        self.agent_name = harness.agent_name().to_string();
        self.load_history(harness.history());

        let mut guard = TerminalGuard::enter()?;

        loop {
            guard.terminal.draw(|frame| Render::draw(self, frame))?;

            if !event::poll(Duration::from_millis(100))? {
                continue;
            }

            let key = match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => key,
                Event::Mouse(mouse) => {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => Input::scroll_history_up(self, 3),
                        MouseEventKind::ScrollDown => Input::scroll_history_down(self, 3),
                        _ => {}
                    }
                    continue;
                }
                _ => continue,
            };

            match key.code {
                KeyCode::PageUp => {
                    Input::scroll_history_up(self, Input::history_page_scroll_amount(self));
                    continue;
                }
                KeyCode::PageDown => {
                    Input::scroll_history_down(self, Input::history_page_scroll_amount(self));
                    continue;
                }
                _ => {}
            }

            if Commands::handle_mode(self, key, &mut guard, harness, config)
                .await
                .is_err()
            {
                return Ok(());
            } else if let InputMode::Exit = self.mode {
                return Ok(());
            }
        }
    }

    pub async fn send_prompt(
        &mut self,
        prompt: String,
        harness: &mut Harness,
        guard: &mut TerminalGuard,
    ) -> Result<(), Box<dyn Error>> {
        Input::clear_input(self);
        Input::reset_history_scroll(self);
        self.messages.push(TuiMessage {
            role: MessageRole::User,
            text: prompt.clone(),
            is_markdown: false,
        });
        self.status = "Waiting for agent...".to_string();
        guard.terminal.draw(|frame| Render::draw(self, frame))?;

        let result = harness
            .run_turn_with_events(prompt, |event| {
                match event {
                    HarnessEvent::AgentMessageChunk(text) => {
                        self.append_agent_message_chunk(text);
                    }
                    HarnessEvent::ToolCall { name, arguments } => {
                        self.messages.push(TuiMessage {
                            role: MessageRole::ToolCall,
                            text: format!("{} ({})", name, arguments),
                            is_markdown: false,
                        });
                        Input::reset_history_scroll(self);
                    }
                }
                let _ = guard.terminal.draw(|frame| Render::draw(self, frame));
            })
            .await;

        match result {
            Ok(()) => {
                self.status = "Ctrl-C to quit".to_string();
                self.context_tokens = harness.context_tokens();
            }
            Err(err) => {
                self.messages.push(TuiMessage {
                    role: MessageRole::Agent,
                    text: format!("Error: {}", err),
                    is_markdown: false,
                });
                Input::reset_history_scroll(self);
                self.status = "Agent request failed".to_string();
            }
        };

        Ok(())
    }

    pub fn append_agent_message_chunk(&mut self, chunk: String) {
        if chunk.is_empty() {
            return;
        }

        if let Some(message) = self.messages.last_mut()
            && message.role == MessageRole::Agent
        {
            message.text.push_str(&chunk);
        } else {
            self.messages.push(TuiMessage {
                role: MessageRole::Agent,
                text: chunk,
                is_markdown: true,
            });
        }
        Input::reset_history_scroll(self);
    }

    pub fn load_history(&mut self, history: &ChatRequest) {
        if let Some(system) = &history.system
            && !system.trim().is_empty()
        {
            self.messages.push(TuiMessage {
                role: MessageRole::Agent,
                text: format!("system: {}", system),
                is_markdown: false,
            });
        }

        for message in &history.messages {
            for tui_message in tui_messages_for_chat_message(message.role.clone(), &message.content)
            {
                self.messages.push(tui_message);
            }
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
                    is_markdown: matches!(role, ChatRole::Assistant),
                });
            }
            ContentPart::ToolCall(tool_call) => {
                messages.push(TuiMessage {
                    role: MessageRole::ToolCall,
                    text: format!("{} ({})", tool_call.fn_name, tool_call.fn_arguments),
                    is_markdown: false,
                });
            }
            ContentPart::ToolResponse(tool_response) => {
                messages.push(TuiMessage {
                    role: MessageRole::ToolCall,
                    text: format!("{} => {}", tool_response.call_id, tool_response.content),
                    is_markdown: false,
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
    pub fn parts(&self) -> (&'static str, Style) {
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

pub fn wrapped_line_count(lines: &[Line<'_>], width: u16) -> u16 {
    let width = width.max(1) as usize;
    lines.iter().fold(0u16, |total, line| {
        let line_width = line.width();
        let wrapped = (line_width / width) + usize::from(line_width % width != 0);
        total.saturating_add(wrapped.max(1) as u16)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use genai::chat::{ChatMessage, MessageContent, ToolCall, ToolResponse};

    #[test]
    fn counts_wrapped_history_lines() {
        let lines = vec![Line::from("12345"), Line::from("123456")];

        assert_eq!(wrapped_line_count(&lines, 5), 3);
        assert_eq!(wrapped_line_count(&lines, 0), 11);
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
