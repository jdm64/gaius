/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::Agents,
    config::Config,
    diff_view::DiffView,
    harness::{Harness, HarnessEvent, HarnessSnapshot},
    harness_actor::{HarnessActorEvent, HarnessActorHandle},
    input::{InputMode, PromptEditor},
    render::{Render, history::DisplayPrefs, layout::HistoryLayout},
    selection::Selection,
    token_usage::format_arrows,
};
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEventKind,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    error::Error,
    io::{self, Stdout},
    time::Duration,
};
use tokio::time::{self, Instant};

const STREAM_FRAME_INTERVAL: Duration = Duration::from_millis(1000 / 15);
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(1000 / 60);

pub enum RenderReason {
    UserUI,
    HarnessMsg,
    Timer,
}

#[derive(Clone)]
pub enum TuiMessage {
    UserPrompt(String),
    PlanMessage(String),
    AgentMessage(String),
    Thinking(String),
    SystemMessage(String),
    CompactionStart {
        start_time: u64,
    },
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
    pub editor: PromptEditor,
    pub messages: Vec<TuiMessage>,
    pub context_tokens: Option<i32>,
    pub display_prefs: DisplayPrefs,
    pub history: HistoryLayout,
    pub selection: Selection,
    pub actor_busy: bool,
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
            editor: PromptEditor::new(),
            messages: Vec::new(),
            context_tokens: None,
            display_prefs: DisplayPrefs::load().unwrap_or_default(),
            history: HistoryLayout::default(),
            selection: Selection::default(),
            actor_busy: false,
        }
    }

    pub async fn run(&mut self, harness: Harness) -> Result<HarnessSnapshot, Box<dyn Error>> {
        self.agents = self.config.agents().clone();
        self.load_history(&harness);
        if let Err(e) = self.editor.load_prompt_history() {
            eprintln!("Failed to load prompt history: {}", e);
        }
        let mut latest_snapshot = harness.snapshot();
        let mut actor = HarnessActorHandle::new(harness)?;
        self.save_snapshot(&latest_snapshot);

        if latest_snapshot.model.id.is_empty() {
            self.push_message(TuiMessage::SystemMessage(
                "No model selected, use /models to configure a model first".to_string(),
            ));
        }

        let mut guard = TerminalGuard::enter()?;
        let mut terminal_events = EventStream::new();
        let render = Render::new();
        let mut render_reason;
        let mut last_render: Instant = Instant::now();
        let mut next_render: Option<Instant> = Some(last_render + STREAM_FRAME_INTERVAL);

        loop {
            tokio::select! {
                event = terminal_events.next() => {
                    let Some(event) = event else {
                        break;
                    };
                    self.handle_terminal_event(event?, &actor).await?;
                    render_reason = RenderReason::UserUI;
                }
                actor_event = actor.rx.recv() => {
                    let Some(actor_event) = actor_event else {
                        break;
                    };

                    render_reason = match &actor_event {
                        HarnessActorEvent::Harness(HarnessEvent::QueueChanged(_)) => {
                            RenderReason::UserUI
                        }
                        HarnessActorEvent::Harness(_) => RenderReason::HarnessMsg,
                        _ => RenderReason::UserUI,
                    };

                    if let Some(snapshot) = self.handle_actor_event(actor_event) {
                        latest_snapshot = snapshot;
                    }
                }
                _ = time::sleep_until(next_render.unwrap_or(Instant::now())), if next_render.is_some() => {
                    render_reason = RenderReason::Timer;
                }
            }

            if let InputMode::Exit = self.editor.mode {
                break;
            }

            let should_draw = match render_reason {
                RenderReason::Timer => true,
                RenderReason::HarnessMsg => false,
                RenderReason::UserUI => last_render.elapsed() >= MIN_FRAME_INTERVAL,
            };

            if should_draw {
                guard.terminal.draw(|frame| render.draw(self, frame))?;
                last_render = Instant::now();
                next_render = if self.actor_busy {
                    Some(Instant::now() + STREAM_FRAME_INTERVAL)
                } else {
                    None
                };
            } else {
                // based on should_draw there are only two cases why we didn't draw
                // if it was HarnessMsg then schedule draw at 15 fps
                // if it was UserUI then schedule draw at 60 fps
                // other cases shouldn't happen so set to idle(None)
                next_render = match render_reason {
                    RenderReason::UserUI => Some(last_render + MIN_FRAME_INTERVAL),
                    RenderReason::HarnessMsg => Some(last_render + STREAM_FRAME_INTERVAL),
                    _ => None,
                };
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
                        self.history.scroll_up(3);
                    }
                    MouseEventKind::ScrollDown => {
                        self.selection.selection = None;
                        self.history.scroll_down(3);
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        self.selection.mouse_down(mouse);
                    }
                    MouseEventKind::Drag(MouseButton::Left) => {
                        self.selection.mouse_drag(mouse);
                    }
                    MouseEventKind::Up(MouseButton::Left) => {
                        if let Some(status) = self.selection.mouse_up(mouse) {
                            self.editor.status = status;
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
                let amount = self.history.scroll_size();
                self.history.scroll_up(amount);
                return Ok(());
            }
            KeyCode::PageDown => {
                let amount = self.history.scroll_size();
                self.history.scroll_down(amount);
                return Ok(());
            }
            _ => {}
        }

        if matches!(self.editor.mode, InputMode::Question { .. }) {
            self.editor.handle_question_key(key);
        } else {
            InputMode::handle_mode(self, key, actor).await?;
        }

        Ok(())
    }

    pub async fn queue_prompt(
        &mut self,
        prompt: String,
        actor: &HarnessActorHandle,
    ) -> Result<(), Box<dyn Error>> {
        self.agents.mark_recent(&self.snapshot.agent_name);
        self.editor.update_prompt_history(prompt.clone());
        self.editor.status_clear_input("Waiting for agent...");
        self.history.scroll_bottom();

        if let Err(err) = actor.run_prompt(prompt).await {
            self.push_message(TuiMessage::SystemMessage(format!("Error: {}", err)));
            self.editor.status = "Agent request failed".to_string();
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
                self.editor.clear_input();
                self.editor.answer_tx = Some(answer_tx);
                self.editor.mode = InputMode::Question {
                    title,
                    options,
                    selected: 0,
                };
                None
            }
            HarnessActorEvent::TurnFinished(snapshot) => {
                self.actor_busy = false;
                self.finish_last_timer();
                self.save_snapshot(&snapshot);
                self.editor.status = "".to_string();
                Some(snapshot)
            }
            HarnessActorEvent::RequestFailed(err, snapshot) => {
                self.actor_busy = false;
                self.finish_last_timer();
                self.save_snapshot(&snapshot);
                self.push_message(TuiMessage::SystemMessage(format!("Error: {}", err)));
                self.editor.status = "Agent request failed".to_string();
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

    pub fn save_snapshot(&mut self, snapshot: &HarnessSnapshot) {
        self.snapshot = snapshot.clone();
    }

    pub fn harness_idle(&self) -> bool {
        !self.actor_busy && self.snapshot.queued_prompts == 0
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
            HarnessEvent::CompactStart { start_time } => {
                self.push_message(TuiMessage::CompactionStart { start_time });
            }
            HarnessEvent::CompactSummary(text) => {
                self.finish_last_timer();
                self.push_message(TuiMessage::AgentMessage(text));
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
                self.finish_last_timer();
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
                self.snapshot.turn_started = Some(turn_started);
                self.editor.status = "Waiting for agent...".to_string();
            }
            HarnessEvent::TurnDuration(duration_ms) => {
                self.push_message(TuiMessage::TurnDuration(duration_ms));
            }
            HarnessEvent::QueueChanged(count) => {
                self.snapshot.queued_prompts = count;
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

        self.set_dirty_from(self.messages.len().saturating_sub(1));
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

        self.set_dirty_from(self.messages.len().saturating_sub(1));
    }

    pub fn load_history(&mut self, harness: &Harness) {
        self.clear_messages();
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
            HarnessEvent::CompactStart { start_time } => {
                self.push_message(TuiMessage::CompactionStart { start_time });
            }
            HarnessEvent::CompactSummary(text) => {
                self.push_message(TuiMessage::AgentMessage(text));
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
                self.finish_last_timer();
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
            HarnessEvent::QueueChanged(_) => {}
        });
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.history.clear();
    }

    pub fn toggle_thinking(&mut self) {
        self.editor.status = self.display_prefs.toggle_thinking();
        self.save_display_prefs();
        self.history.invalidate_from(0);
    }

    pub fn toggle_token_info(&mut self) {
        self.editor.status = self.display_prefs.toggle_token_info();
        self.save_display_prefs();
        self.history.invalidate_from(0);
    }

    pub fn toggle_diff_view(&mut self) {
        self.editor.status = self.display_prefs.toggle_diff_view();
        self.save_display_prefs();
        self.history.invalidate_from(0);
    }

    fn save_display_prefs(&self) {
        if let Err(e) = self.display_prefs.save() {
            eprintln!("Failed to save display prefs: {}", e);
        }
    }

    pub fn push_message(&mut self, message: TuiMessage) {
        let mut removed_padding = false;
        if !matches!(message, TuiMessage::Padding)
            && matches!(self.messages.last(), Some(TuiMessage::Padding))
        {
            self.messages.pop();
            removed_padding = true;
        }

        let idx = self.messages.len();
        self.messages.push(message);
        self.history.begin_message_block(removed_padding);
        self.set_dirty_from(idx);
    }

    fn set_dirty_from(&mut self, idx: usize) {
        self.history.invalidate_from(idx);
    }

    fn finish_last_timer(&mut self) {
        for (idx, message) in self.messages.iter_mut().enumerate().rev() {
            let start_time = match message {
                TuiMessage::ToolCall { start_time, .. } => start_time,
                TuiMessage::CompactionStart { start_time } => start_time,
                _ => continue,
            };
            *start_time = 0;
            self.set_dirty_from(idx);
            break;
        }
    }
}
