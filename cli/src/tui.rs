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
    util::cache_dir,
};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use genai::chat::{ChatRequest, ChatRole, ContentPart};
use ratatui::{Terminal, backend::CrosstermBackend, text::Line};
use std::{
    error::Error,
    fs,
    io::{self, Stdout},
    path::PathBuf,
};

#[derive(Clone)]
pub struct TuiMessage {
    pub role: MessageRole,
    pub text: String,
}

#[derive(Debug, PartialEq, Clone)]
pub enum MessageRole {
    User,
    Agent,
    ToolCall,
    System,
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
    pub prompt_history: Vec<String>,
    pub prompt_history_idx: Option<usize>,
    pub history_lines: Vec<Line<'static>>,
    pub history_generation: u64,
    pub rendered_history_generation: u64,
}

impl Default for TuiApp {
    fn default() -> Self {
        Self::new()
    }
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
            prompt_history: Vec::new(),
            prompt_history_idx: None,
            history_lines: Vec::new(),
            history_generation: 0,
            rendered_history_generation: u64::MAX,
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
        if let Err(e) = self.load_prompt_history() {
            eprintln!("Failed to load prompt history: {}", e);
        }

        let mut guard = TerminalGuard::enter()?;
        guard.terminal.draw(|frame| Render::draw(self, frame))?;

        loop {
            let key = match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => key,
                Event::Mouse(mouse) => {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => Input::scroll_history_up(self, 3),
                        MouseEventKind::ScrollDown => Input::scroll_history_down(self, 3),
                        _ => continue,
                    }
                    guard.terminal.draw(|frame| Render::draw(self, frame))?;
                    continue;
                }
                Event::Resize(_, _) => {
                    guard.terminal.draw(|frame| Render::draw(self, frame))?;
                    continue;
                }
                _ => continue,
            };

            match key.code {
                KeyCode::PageUp => {
                    Input::scroll_history_up(self, Input::history_page_scroll_amount(self));
                    guard.terminal.draw(|frame| Render::draw(self, frame))?;
                    continue;
                }
                KeyCode::PageDown => {
                    Input::scroll_history_down(self, Input::history_page_scroll_amount(self));
                    guard.terminal.draw(|frame| Render::draw(self, frame))?;
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

            guard.terminal.draw(|frame| Render::draw(self, frame))?;
        }
    }

    pub async fn send_prompt(
        &mut self,
        prompt: String,
        harness: &mut Harness,
        guard: &mut TerminalGuard,
    ) -> Result<(), Box<dyn Error>> {
        Input::update_prompt_history(self, prompt.clone());
        Input::clear_input(self);
        Input::reset_history_scroll(self);
        self.push_message(TuiMessage {
            role: MessageRole::User,
            text: prompt.clone(),
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
                        self.push_message(TuiMessage {
                            role: MessageRole::ToolCall,
                            text: format!("{} ({})", name, arguments),
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
                self.push_message(TuiMessage {
                    role: MessageRole::System,
                    text: format!("Error: {}", err),
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
            self.push_message(TuiMessage {
                role: MessageRole::Agent,
                text: chunk,
            });
        }
        self.mark_history_dirty();
        Input::reset_history_scroll(self);
    }

    pub fn load_history(&mut self, history: &ChatRequest) {
        if let Some(system) = &history.system
            && !system.trim().is_empty()
        {
            self.push_message(TuiMessage {
                role: MessageRole::System,
                text: format!("system: {}", system),
            });
        }

        for message in &history.messages {
            for tui_message in tui_messages_for_chat_message(message.role.clone(), &message.content)
            {
                self.push_message(tui_message);
            }
        }
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.mark_history_dirty();
    }

    pub fn push_message(&mut self, message: TuiMessage) {
        self.messages.push(message);
        self.mark_history_dirty();
    }

    pub fn mark_history_dirty(&mut self) {
        self.history_generation = self.history_generation.wrapping_add(1);
    }

    pub fn prompt_history_file() -> Result<PathBuf, Box<dyn Error>> {
        Ok(cache_dir()?.join("prompt_history.json"))
    }

    pub fn load_prompt_history(&mut self) -> Result<(), Box<dyn Error>> {
        let path = Self::prompt_history_file()?;
        if path.exists() {
            let contents = fs::read_to_string(&path)?;
            self.prompt_history = serde_json::from_str(&contents).unwrap_or_default();
        }
        self.prompt_history_idx = None;
        Ok(())
    }

    pub fn save_prompt_history(&self) -> Result<(), Box<dyn Error>> {
        let path = Self::prompt_history_file()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(&self.prompt_history)?;
        fs::write(path, contents)?;
        Ok(())
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

pub fn wrapped_line_count(lines: &[Line<'_>], width: u16) -> u16 {
    let width = width.max(1) as usize;
    lines.iter().fold(0u16, |total, line| {
        let line_width = line.width();
        let wrapped = (line_width / width) + usize::from(line_width % width != 0);
        total.saturating_add(wrapped.max(1) as u16)
    })
}
