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
    agents::Agents,
    commands::Commands,
    config::Config,
    harness::{Harness, HarnessEvent, HarnessSnapshot},
    harness_actor::{HarnessActorEvent, HarnessActorHandle},
    input::{Input, InputMode},
    models::ModelDef,
    render::Render,
    token_usage::format_arrows,
    util::cache_dir,
};
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend, text::Line};
use std::{
    error::Error,
    fs,
    io::{self, Stdout},
    path::PathBuf,
};
use tokio::sync::oneshot;

#[derive(Clone)]
pub enum TuiMessage {
    UserPrompt(String),
    AgentMessage(String),
    Thinking(String),
    SystemMessage(String),
    TokenInfo(String),
    ToolCall {
        name: String,
        arguments: String,
        result: String,
        error: bool,
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
    pub config: Config,
    pub model: ModelDef,
    pub agent_name: String,
    pub agents: Agents,
    pub input: String,
    pub input_cursor: usize,
    pub history_scroll: u16,
    pub history_page_size: u16,
    pub messages: Vec<TuiMessage>,
    pub status: String,
    pub mode: InputMode,
    pub context_tokens: Option<i32>,
    pub show_thinking: bool,
    pub show_token_info: bool,
    pub prompt_history: Vec<String>,
    pub prompt_history_idx: Option<usize>,
    pub history_lines: Vec<Line<'static>>,
    pub history_generation: u64,
    pub rendered_history_generation: u64,
    pub actor_busy: bool,
    pub queued_prompts: usize,
    pub question_answer_tx: Option<oneshot::Sender<String>>,
}

impl Default for TuiApp {
    fn default() -> Self {
        Self::new(Config::default())
    }
}

impl TuiApp {
    pub fn new(config: Config) -> Self {
        let agents = config.agents().clone();
        Self {
            config,
            model: ModelDef {
                provider: String::new(),
                id: String::new(),
                context_len: None,
            },
            agent_name: String::new(),
            agents,
            input: String::new(),
            input_cursor: 0,
            history_scroll: 0,
            history_page_size: 1,
            messages: Vec::new(),
            status: "".to_string(),
            mode: InputMode::PromptInput,
            context_tokens: None,
            show_thinking: false,
            show_token_info: true,
            prompt_history: Vec::new(),
            prompt_history_idx: None,
            history_lines: Vec::new(),
            history_generation: 0,
            rendered_history_generation: u64::MAX,
            actor_busy: false,
            queued_prompts: 0,
            question_answer_tx: None,
        }
    }

    pub async fn run(&mut self, harness: Harness) -> Result<HarnessSnapshot, Box<dyn Error>> {
        self.agents = self.config.agents().clone();
        self.load_history(&harness);
        if let Err(e) = self.load_prompt_history() {
            eprintln!("Failed to load prompt history: {}", e);
        }
        let mut latest_snapshot = harness.snapshot();
        let mut actor = HarnessActorHandle::new(harness);
        self.apply_snapshot(&latest_snapshot);

        let mut guard = TerminalGuard::enter()?;
        let mut terminal_events = EventStream::new();
        let render = Render::new();
        guard.terminal.draw(|frame| render.draw(self, frame))?;

        loop {
            tokio::select! {
                event = terminal_events.next() => {
                    let Some(event) = event else {
                        break;
                    };
                    self.handle_terminal_event(event?, &actor).await?;
                }
                actor_event = actor.rx.recv() => {
                    let Some(actor_event) = actor_event else {
                        break;
                    };
                    if let Some(snapshot) = self.handle_actor_event(actor_event) {
                        latest_snapshot = snapshot;
                    }
                }
            }

            if let InputMode::Exit = self.mode {
                break;
            }

            guard.terminal.draw(|frame| render.draw(self, frame))?;
        }

        if self.actor_busy {
            Ok(latest_snapshot)
        } else {
            match actor.shutdown().await {
                Ok(snapshot) => Ok(snapshot),
                Err(_) => Ok(latest_snapshot),
            }
        }
    }

    async fn handle_terminal_event(
        &mut self,
        event: Event,
        actor: &HarnessActorHandle,
    ) -> Result<(), Box<dyn Error>> {
        let key = match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => key,
            Event::Mouse(mouse) => {
                match mouse.kind {
                    MouseEventKind::ScrollUp => Input::scroll_history_up(self, 3),
                    MouseEventKind::ScrollDown => Input::scroll_history_down(self, 3),
                    _ => {}
                }
                return Ok(());
            }
            Event::Resize(_, _) => return Ok(()),
            _ => return Ok(()),
        };

        match key.code {
            KeyCode::PageUp => {
                Input::scroll_history_up(self, Input::history_page_scroll_amount(self));
                return Ok(());
            }
            KeyCode::PageDown => {
                Input::scroll_history_down(self, Input::history_page_scroll_amount(self));
                return Ok(());
            }
            _ => {}
        }

        if matches!(self.mode, InputMode::Question { .. }) {
            self.handle_question_key(key);
        } else {
            Commands::handle_mode(self, key, actor).await?;
        }

        Ok(())
    }

    pub async fn queue_prompt(
        &mut self,
        prompt: String,
        actor: &HarnessActorHandle,
    ) -> Result<(), Box<dyn Error>> {
        self.agents.mark_recent(&self.agent_name);
        Input::update_prompt_history(self, prompt.clone());
        Input::clear_input(self);
        Input::reset_history_scroll(self);
        self.queued_prompts += 1;
        self.status = if self.actor_busy {
            format!("Queued prompt ({} pending)", self.queued_prompts)
        } else {
            "Waiting for agent...".to_string()
        };

        if let Err(err) = actor.run_prompt(prompt).await {
            self.queued_prompts = self.queued_prompts.saturating_sub(1);
            self.push_message(TuiMessage::SystemMessage(format!("Error: {}", err)));
            self.status = "Agent request failed".to_string();
        }

        Ok(())
    }

    fn handle_actor_event(&mut self, event: HarnessActorEvent) -> Option<HarnessSnapshot> {
        match event {
            HarnessActorEvent::Harness(event) => {
                self.apply_harness_event(event);
                None
            }
            HarnessActorEvent::AskUser {
                title,
                mut options,
                answer_tx,
            } => {
                Input::clear_input(self);
                Input::reset_history_scroll(self);
                options.push("Other:".to_string());
                self.question_answer_tx = Some(answer_tx);
                self.mode = InputMode::Question {
                    title,
                    options,
                    selected: 0,
                };
                None
            }
            HarnessActorEvent::TurnStarted => {
                self.actor_busy = true;
                self.queued_prompts = self.queued_prompts.saturating_sub(1);
                self.status = "Waiting for agent...".to_string();
                None
            }
            HarnessActorEvent::TurnFinished(snapshot) => {
                self.actor_busy = false;
                self.apply_snapshot(&snapshot);
                self.status = if self.queued_prompts > 0 {
                    format!("Queued prompt ({} pending)", self.queued_prompts)
                } else {
                    "".to_string()
                };
                Some(snapshot)
            }
            HarnessActorEvent::RequestFailed(err, snapshot) => {
                self.actor_busy = false;
                self.apply_snapshot(&snapshot);
                self.push_message(TuiMessage::SystemMessage(format!("Error: {}", err)));
                Input::reset_history_scroll(self);
                self.status = "Agent request failed".to_string();
                Some(snapshot)
            }
            HarnessActorEvent::HistoryReplayed(events) => {
                self.clear_messages();
                for event in events {
                    self.apply_harness_event(event);
                }
                None
            }
        }
    }

    fn handle_question_key(&mut self, key: KeyEvent) {
        let mode = std::mem::replace(&mut self.mode, InputMode::PromptInput);
        let InputMode::Question {
            title,
            options,
            mut selected,
        } = mode
        else {
            self.mode = mode;
            return;
        };

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.answer_question(String::new());
                self.mode = InputMode::Exit;
            }
            KeyCode::Esc | KeyCode::Tab => {
                self.answer_question(String::new());
                Input::clear_input(self);
                self.mode = InputMode::PromptInput;
            }
            KeyCode::Enter => {
                let answer = options.get(selected).cloned().unwrap_or_default();
                let details = self.input.trim().to_string();
                let response = [answer, details]
                    .iter()
                    .filter(|part| !part.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
                self.answer_question(response);
                Input::clear_input(self);
                self.mode = InputMode::PromptInput;
            }
            KeyCode::Up => {
                selected = selected.saturating_sub(1);
                self.mode = InputMode::Question {
                    title,
                    options,
                    selected,
                };
            }
            KeyCode::Down => {
                if selected + 1 < options.len() {
                    selected += 1;
                }
                self.mode = InputMode::Question {
                    title,
                    options,
                    selected,
                };
            }
            _ => {
                Input::handle_input_cursor(self, key);
                self.mode = InputMode::Question {
                    title,
                    options,
                    selected,
                };
            }
        }
    }

    fn answer_question(&mut self, answer: String) {
        if let Some(answer_tx) = self.question_answer_tx.take() {
            let _ = answer_tx.send(answer);
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: &HarnessSnapshot) {
        self.model = snapshot.model.clone();
        self.agent_name = snapshot.agent_name.clone();
    }

    pub fn harness_idle(&self) -> bool {
        !self.actor_busy && self.queued_prompts == 0
    }

    fn apply_harness_event(&mut self, event: HarnessEvent) {
        match event {
            HarnessEvent::SystemMessage(text) => {
                self.push_message(TuiMessage::SystemMessage(text));
            }
            HarnessEvent::UserPrompt(prompt_msg) => {
                self.push_message(TuiMessage::UserPrompt(prompt_msg));
            }
            HarnessEvent::AgentMessage(chunk) => {
                self.append_agent_message(chunk, false);
            }
            HarnessEvent::Thinking(chunk) => {
                self.append_agent_message(chunk, true);
            }
            HarnessEvent::ToolCall {
                name,
                arguments,
                result,
                error,
            } => {
                self.push_message(TuiMessage::ToolCall {
                    name,
                    arguments,
                    result,
                    error,
                });
                Input::reset_history_scroll(self);
            }
            HarnessEvent::TokenUsage {
                prompt,
                response,
                total,
            } => {
                let info = format_arrows(prompt, response);
                self.append_token_info(info);
                self.context_tokens = total;
            }
            HarnessEvent::AskUser { .. } => {}
        }
    }

    pub fn append_agent_message(&mut self, chunk: String, thinking: bool) {
        if chunk.is_empty() {
            return;
        }

        match self.messages.last_mut() {
            Some(TuiMessage::AgentMessage(text)) if !thinking => {
                text.push_str(chunk.as_str());
            }
            Some(TuiMessage::Thinking(text)) if thinking => {
                text.push_str(chunk.as_str());
            }
            _ => {
                if thinking {
                    self.push_message(TuiMessage::Thinking(chunk));
                } else {
                    self.push_message(TuiMessage::AgentMessage(chunk));
                }
            }
        }

        self.mark_history_dirty();
        Input::reset_history_scroll(self);
    }

    pub fn append_token_info(&mut self, chunk: String) {
        if chunk.is_empty() {
            return;
        }

        match self.messages.last_mut() {
            Some(TuiMessage::TokenInfo(text)) => {
                text.push(' ');
                text.push_str(chunk.as_str());
            }
            _ => {
                self.push_message(TuiMessage::TokenInfo(chunk));
            }
        }

        self.mark_history_dirty();
        Input::reset_history_scroll(self);
    }

    pub fn load_history(&mut self, harness: &Harness) {
        self.messages.clear();
        harness.replay_history(|event| match event {
            HarnessEvent::SystemMessage(text) => {
                self.push_message(TuiMessage::SystemMessage(text));
            }
            HarnessEvent::UserPrompt(text) => {
                self.push_message(TuiMessage::UserPrompt(text));
            }
            HarnessEvent::AgentMessage(text) => {
                self.append_agent_message(text, false);
            }
            HarnessEvent::Thinking(text) => {
                self.append_agent_message(text, true);
            }
            HarnessEvent::ToolCall {
                name,
                arguments,
                result,
                error,
            } => {
                self.push_message(TuiMessage::ToolCall {
                    name,
                    arguments,
                    result,
                    error,
                });
            }
            HarnessEvent::TokenUsage {
                prompt,
                response,
                total,
            } => {
                let info = format_arrows(prompt, response);
                self.append_token_info(info);
                self.context_tokens = total;
            }
            HarnessEvent::AskUser {
                title: _,
                options: _,
            } => {}
        });
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.mark_history_dirty();
    }

    pub fn toggle_thinking(&mut self) {
        self.show_thinking = !self.show_thinking;
        self.status = format!(
            "Thinking display: {}",
            if self.show_thinking { "on" } else { "off" }
        );
        self.mark_history_dirty();
    }

    pub fn toggle_token_info(&mut self) {
        self.show_token_info = !self.show_token_info;
        self.status = format!(
            "Token info display: {}",
            if self.show_token_info { "on" } else { "off" }
        );
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
