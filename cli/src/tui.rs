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
use ratatui::{Terminal, backend::CrosstermBackend, text::Line};
use std::{
    error::Error,
    fs,
    io::{self, Stdout},
    path::PathBuf,
};

#[derive(Clone)]
pub enum TuiMessage {
    UserPrompt(String),
    AgentMessage(String),
    SystemMessage(String),
    ToolCall {
        name: String,
        arguments: String,
        result: String,
    },
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
        self.load_history(harness);
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
        self.status = "Waiting for agent...".to_string();
        guard.terminal.draw(|frame| Render::draw(self, frame))?;

        let result = harness
            .run_turn_with_events(prompt, |event| {
                match event {
                    HarnessEvent::UserPrompt(prompt_msg) => {
                        self.push_message(TuiMessage::UserPrompt(prompt_msg));
                    }
                    HarnessEvent::AgentMessage(chunk) => {
                        self.append_agent_message(chunk);
                    }
                    HarnessEvent::ToolCall {
                        name,
                        arguments,
                        result,
                    } => {
                        self.push_message(TuiMessage::ToolCall {
                            name,
                            arguments,
                            result,
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
                self.push_message(TuiMessage::SystemMessage(format!("Error: {}", err)));
                Input::reset_history_scroll(self);
                self.status = "Agent request failed".to_string();
            }
        };

        Ok(())
    }

    pub fn append_agent_message(&mut self, chunk: String) {
        if chunk.is_empty() {
            return;
        }

        if let Some(message) = self.messages.last_mut()
            && matches!(message, TuiMessage::AgentMessage(_))
        {
            if let TuiMessage::AgentMessage(text) = message {
                text.push_str(&chunk);
            }
        } else {
            self.push_message(TuiMessage::AgentMessage(chunk));
        }
        self.mark_history_dirty();
        Input::reset_history_scroll(self);
    }

    pub fn load_history(&mut self, harness: &Harness) {
        self.messages.clear();
        harness.replay_history(|event| match event {
            HarnessEvent::UserPrompt(text) => {
                self.push_message(TuiMessage::UserPrompt(text));
            }
            HarnessEvent::AgentMessage(text) => {
                self.append_agent_message(text);
            }
            HarnessEvent::ToolCall {
                name, arguments, result, ..
            } => {
                self.push_message(TuiMessage::ToolCall {
                    name,
                    arguments,
                    result,
                });
            }
        });
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

pub fn wrapped_line_count(lines: &[Line<'_>], width: u16) -> u16 {
    let width = width.max(1) as usize;
    lines.iter().fold(0u16, |total, line| {
        let line_width = line.width();
        let wrapped = (line_width / width) + usize::from(line_width % width != 0);
        total.saturating_add(wrapped.max(1) as u16)
    })
}
