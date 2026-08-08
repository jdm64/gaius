/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::Agents,
    commands::Commands,
    config::Config,
    diff_view::DiffView,
    dirs::Dirs,
    harness::{Harness, HarnessEvent, HarnessSnapshot},
    harness_actor::{HarnessActorEvent, HarnessActorHandle},
    input::{Input, InputMode},
    render::Render,
    render_history::DisplayPrefs,
    selection::Selection,
    token_usage::format_arrows,
};
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
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
    time::Duration,
};
use tokio::{sync::oneshot, time};

const MAX_FPS: u64 = 60;
const FRAME_INTERVAL_MS: u64 = 1000 / MAX_FPS;

#[derive(Clone)]
pub enum TuiMessage {
    UserPrompt(String),
    PlanMessage(String),
    AgentMessage(String),
    Thinking(String),
    SystemMessage(String),
    TokenInfo(String),
    ToolCall {
        name: String,
        arguments: String,
        start_time: u64,
    },
    ToolResult {
        name: String,
        result: String,
        error: bool,
    },
    DiffView(DiffView),
    TurnDuration(u64),
    Padding,
}

/// Bookkeeping for a tool-call duration line that must keep advancing each
/// frame while the tool is running. `line_index` is the position in
/// `TuiApp::history_lines`; `start_time` is the tool call's start timestamp.
pub struct LiveTimer {
    pub line_index: usize,
    pub start_time: u64,
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
    pub snapshot: HarnessSnapshot,
    pub agents: Agents,
    pub input: String,
    pub input_cursor: usize,
    pub history_scroll: u16,
    pub history_page_size: u16,
    pub history_height: u16,
    pub new_lines_below: u16,
    pub messages: Vec<TuiMessage>,
    pub status: String,
    pub mode: InputMode,
    pub context_tokens: Option<i32>,
    pub display_prefs: DisplayPrefs,
    pub prompt_history: Vec<String>,
    pub prompt_history_idx: Option<usize>,
    pub history_lines: Vec<Line<'static>>,
    pub live_timers: Vec<LiveTimer>,
    pub selection: Selection,
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
            snapshot: HarnessSnapshot::default(),
            agents,
            input: String::new(),
            input_cursor: 0,
            history_scroll: 0,
            history_page_size: 1,
            history_height: 0,
            new_lines_below: 0,
            messages: Vec::new(),
            status: "".to_string(),
            mode: InputMode::PromptInput,
            context_tokens: None,
            display_prefs: DisplayPrefs {
                thinking: false,
                token_info: true,
                diff_view: true,
            },
            prompt_history: Vec::new(),
            prompt_history_idx: None,
            history_lines: Vec::new(),
            live_timers: Vec::new(),
            selection: Selection::default(),
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
        self.save_snapshot(&latest_snapshot);

        let mut guard = TerminalGuard::enter()?;
        let mut terminal_events = EventStream::new();
        let mut frame_timer = time::interval(Duration::from_millis(FRAME_INTERVAL_MS));
        let render = Render::new();
        guard.terminal.draw(|frame| render.draw(self, frame))?;
        let mut redraw_pending = false;

        loop {
            tokio::select! {
                event = terminal_events.next() => {
                    let Some(event) = event else {
                        break;
                    };
                    self.handle_terminal_event(event?, &actor).await?;
                    redraw_pending = true;
                }
                actor_event = actor.rx.recv() => {
                    let Some(actor_event) = actor_event else {
                        break;
                    };
                    if let Some(snapshot) = self.handle_actor_event(actor_event) {
                        latest_snapshot = snapshot;
                    }
                    redraw_pending = true;
                }
                _ = frame_timer.tick(), if redraw_pending || !self.live_timers.is_empty() => {
                    guard.terminal.draw(|frame| render.draw(self, frame))?;
                    redraw_pending = false;
                }
            }

            if let InputMode::Exit = self.mode {
                break;
            }
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
                    MouseEventKind::ScrollUp => {
                        self.selection.selection = None;
                        Input::scroll_history_up(self, 3);
                    }
                    MouseEventKind::ScrollDown => {
                        self.selection.selection = None;
                        Input::scroll_history_down(self, 3);
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        self.selection.mouse_down(mouse);
                    }
                    MouseEventKind::Drag(MouseButton::Left) => {
                        self.selection.mouse_drag(mouse);
                    }
                    MouseEventKind::Up(MouseButton::Left) => {
                        if let Some(status) = self.selection.mouse_up(mouse) {
                            self.status = status;
                        }
                    }
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
        self.agents.mark_recent(&self.snapshot.agent_name);
        Input::update_prompt_history(self, prompt.clone());
        Input::clear_input(self);
        Input::scroll_history_bottom(self);
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
                options,
                answer_tx,
            } => {
                Input::clear_input(self);
                self.question_answer_tx = Some(answer_tx);
                self.mode = InputMode::Question {
                    title,
                    options,
                    selected: 0,
                };
                None
            }
            HarnessActorEvent::TurnFinished(snapshot) => {
                self.actor_busy = false;
                self.finish_last_tool_call();
                self.save_snapshot(&snapshot);
                self.status = if self.queued_prompts > 0 {
                    format!("Queued prompt ({} pending)", self.queued_prompts)
                } else {
                    "".to_string()
                };
                Some(snapshot)
            }
            HarnessActorEvent::RequestFailed(err, snapshot) => {
                self.actor_busy = false;
                self.finish_last_tool_call();
                self.save_snapshot(&snapshot);
                self.push_message(TuiMessage::SystemMessage(format!("Error: {}", err)));
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

    pub fn save_snapshot(&mut self, snapshot: &HarnessSnapshot) {
        self.snapshot = snapshot.clone();
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
            HarnessEvent::PlanMessage(text) => {
                self.push_message(TuiMessage::PlanMessage(text));
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
                start_time,
            } => {
                self.push_message(TuiMessage::ToolCall {
                    name,
                    arguments,
                    start_time,
                });
            }
            HarnessEvent::ToolResult {
                name,
                result,
                error,
            } => {
                self.finish_last_tool_call();
                self.push_message(TuiMessage::ToolResult {
                    name,
                    result,
                    error,
                });
                self.push_message(TuiMessage::Padding);
            }
            HarnessEvent::DiffView(diff) => {
                self.push_message(TuiMessage::DiffView(diff));
            }
            HarnessEvent::TokenUsage {
                prompt,
                response,
                total,
                cost,
            } => {
                let info = format_arrows(prompt, response);
                self.append_token_info(info);
                self.context_tokens = total;
                self.snapshot.total_cost = cost;
            }
            HarnessEvent::AskUser { .. } => {}
            HarnessEvent::TurnStarted(turn_started) => {
                self.actor_busy = true;
                self.queued_prompts = self.queued_prompts.saturating_sub(1);
                self.snapshot.turn_started = Some(turn_started);
                self.status = "Waiting for agent...".to_string();
            }
            HarnessEvent::TurnDuration(duration_ms) => {
                self.push_message(TuiMessage::TurnDuration(duration_ms));
            }
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
            HarnessEvent::PlanMessage(text) => {
                self.push_message(TuiMessage::PlanMessage(text));
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
                start_time,
            } => {
                self.push_message(TuiMessage::ToolCall {
                    name,
                    arguments,
                    start_time,
                });
            }
            HarnessEvent::ToolResult {
                name,
                result,
                error,
            } => {
                self.finish_last_tool_call();
                self.push_message(TuiMessage::ToolResult {
                    name,
                    result,
                    error,
                });
            }
            HarnessEvent::DiffView(diff) => {
                self.push_message(TuiMessage::DiffView(diff));
            }
            HarnessEvent::TokenUsage {
                prompt,
                response,
                total,
                cost,
            } => {
                let info = format_arrows(prompt, response);
                self.append_token_info(info);
                self.context_tokens = total;
                self.snapshot.total_cost = cost;
            }
            HarnessEvent::AskUser {
                title: _,
                options: _,
            } => {}
            HarnessEvent::TurnStarted(_) => {}
            HarnessEvent::TurnDuration(duration_ms) => {
                self.push_message(TuiMessage::TurnDuration(duration_ms));
            }
        });
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.mark_history_dirty();
    }

    pub fn toggle_thinking(&mut self) {
        self.status = self.display_prefs.toggle_thinking();
        self.mark_history_dirty();
    }

    pub fn toggle_token_info(&mut self) {
        self.status = self.display_prefs.toggle_token_info();
        self.mark_history_dirty();
    }

    pub fn toggle_diff_view(&mut self) {
        self.status = self.display_prefs.toggle_diff_view();
        self.mark_history_dirty();
    }

    pub fn push_message(&mut self, message: TuiMessage) {
        if !matches!(message, TuiMessage::Padding) {
            while matches!(self.messages.last(), Some(TuiMessage::Padding)) {
                self.messages.pop();
            }
        }
        self.messages.push(message);
        self.mark_history_dirty();
    }

    fn finish_last_tool_call(&mut self) {
        for message in self.messages.iter_mut().rev() {
            if let TuiMessage::ToolCall { start_time, .. } = message {
                *start_time = 0;
                self.mark_history_dirty();
                break;
            }
        }
    }

    pub fn mark_history_dirty(&mut self) {
        self.history_generation = self.history_generation.wrapping_add(1);
    }

    pub fn load_prompt_history(&mut self) -> Result<(), Box<dyn Error>> {
        let path = Dirs::prompt_history_file()?;
        if path.exists() {
            let contents = fs::read_to_string(&path)?;
            self.prompt_history = serde_json::from_str(&contents).unwrap_or_default();
        }
        self.prompt_history_idx = None;
        Ok(())
    }

    pub fn save_prompt_history(&self) -> Result<(), Box<dyn Error>> {
        let path = Dirs::prompt_history_file()?;
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
