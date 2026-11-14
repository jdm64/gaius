/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    actor::{HarnessActorClient, HarnessCommand, HarnessEvent},
    agents::Agents,
    config::Config,
    diff_view::DiffView,
    harness::{Harness, HarnessMsg, HarnessSnapshot},
    input::{InputMode, PromptEditor},
    render::{Render, history::DisplayPrefs, layout::HistoryLayout},
    selection::Selection,
    token_usage::format_arrows,
    tool_engine::ToolStatus,
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
        status: ToolStatus,
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
            display_prefs: DisplayPrefs::default(),
            history: HistoryLayout::default(),
            selection: Selection::default(),
        }
    }

    pub async fn run(&mut self, harness: Harness) -> Result<HarnessSnapshot, Box<dyn Error>> {
        self.display_prefs = DisplayPrefs::load().unwrap_or_default();
        self.agents = self.config.agents().clone();
        harness.replay_history(|event| self.apply_harness_msg(event));

        if let Err(e) = self.editor.load_prompt_history() {
            eprintln!("Failed to load prompt history: {}", e);
        }
        let initial_snapshot = harness.snapshot();
        let (actor, mut actor_rx) = HarnessActorClient::new(harness)?;
        self.save_snapshot(&initial_snapshot);

        if initial_snapshot.model.id.is_empty() {
            self.push_message(TuiMessage::SystemMessage(
                "No model selected, use /models to configure a model first".to_string(),
            ));
        }

        let mut guard = TerminalGuard::enter()?;
        let mut terminal_events = EventStream::new();
        let mut render = Render::new();
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
                reply = actor_rx.replies.recv() => {
                    let Some(reply) = reply else { break; };
                    if let Some(command) = self.handle_command_reply(reply)
                        && let Err(err) = actor.send(command).await
                    {
                        self.push_message(TuiMessage::SystemMessage(format!("Error: {}", err)));
                    }
                    render_reason = RenderReason::UserUI;
                }
                actor_event = actor_rx.events.recv() => {
                    let Some(actor_event) = actor_event else {
                        break;
                    };

                    render_reason = match &actor_event {
                        HarnessEvent::Message(HarnessMsg::QueueChanged(_)) => {
                            RenderReason::UserUI
                        }
                        HarnessEvent::Message(_) => RenderReason::HarnessMsg,
                        _ => RenderReason::UserUI,
                    };

                    if let Some(command) = self.handle_harness_event(actor_event)
                        && let Err(err) = actor.send(command).await
                    {
                        self.push_message(TuiMessage::SystemMessage(format!("Error: {}", err)));
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
                next_render = if self.snapshot.turn_started.is_some() {
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

        match HarnessActorClient::shutdown(actor.clone_tx(), &mut actor_rx.events).await {
            Some(snapshot) => Ok(snapshot),
            None => Ok(self.snapshot.clone()),
        }
    }

    async fn handle_terminal_event(
        &mut self,
        event: Event,
        actor: &HarnessActorClient,
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
                            self.editor.set_status(status);
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
        actor: &HarnessActorClient,
    ) -> Result<(), Box<dyn Error>> {
        self.agents.mark_recent(&self.snapshot.agent_name);
        self.editor.update_prompt_history(prompt.clone());
        self.editor.status_clear_input("Waiting for agent...");
        self.history.scroll_bottom();

        if let Err(err) = actor.run_prompt(prompt).await {
            self.push_message(TuiMessage::SystemMessage(format!("Error: {}", err)));
            self.editor.set_status("Agent request failed");
        }

        Ok(())
    }

    pub fn handle_harness_event(&mut self, event: HarnessEvent) -> Option<HarnessCommand> {
        match event {
            HarnessEvent::Message(event) => {
                self.apply_harness_msg(event);
            }
            HarnessEvent::AskUser {
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
            }
            HarnessEvent::TurnFinished(snapshot) => {
                self.finish_last_timer();
                self.save_snapshot(&snapshot);
                self.editor.set_status("");
            }
            HarnessEvent::RequestFailed(err, snapshot) => {
                self.finish_last_timer();
                self.save_snapshot(&snapshot);
                self.push_message(TuiMessage::SystemMessage(format!("Error: {}", err)));
                self.editor.set_status("Agent request failed");
            }
            HarnessEvent::ShutdownComplete(result) => {
                if let Ok(snapshot) = result {
                    self.save_snapshot(&snapshot);
                }
            }
        }
        None
    }

    pub fn waiting_for_answer(&self) -> bool {
        matches!(self.editor.mode, InputMode::Question { .. })
    }

    pub fn set_mode(&mut self, mode: InputMode) {
        if self.waiting_for_answer() {
            return;
        }
        self.editor.mode = mode;
    }

    pub fn save_snapshot(&mut self, snapshot: &HarnessSnapshot) {
        self.snapshot = snapshot.clone();
    }

    pub fn apply_harness_msg(&mut self, event: HarnessMsg) {
        match event {
            HarnessMsg::SystemMessage(text) => {
                self.push_message(TuiMessage::SystemMessage(text));
            }
            HarnessMsg::UserPrompt { content, .. } => {
                self.push_message(TuiMessage::UserPrompt(content));
            }
            HarnessMsg::PlanMessage { content, .. } => {
                self.push_message(TuiMessage::PlanMessage(content));
            }
            HarnessMsg::AgentMessage { content, .. } | HarnessMsg::AgentChunk { content, .. } => {
                self.append_agent_message(content, false);
            }
            HarnessMsg::Thinking { content, .. } | HarnessMsg::ThinkingChunk { content, .. } => {
                self.append_agent_message(content, true);
            }
            HarnessMsg::CompactStart { start_time, .. } => {
                self.push_message(TuiMessage::CompactionStart { start_time });
            }
            HarnessMsg::CompactSummary { content, .. } => {
                self.finish_last_timer();
                self.push_message(TuiMessage::AgentMessage(content));
            }
            HarnessMsg::ToolCall {
                name,
                arguments,
                start_time,
                ..
            } => {
                self.push_message(TuiMessage::ToolCall {
                    name,
                    arguments,
                    start_time,
                });
            }
            HarnessMsg::ToolResult {
                name,
                result,
                status,
                ..
            } => {
                self.finish_last_timer();
                self.push_message(TuiMessage::ToolResult {
                    name,
                    result,
                    status,
                });
                self.push_message(TuiMessage::Padding);
            }
            HarnessMsg::DiffView(diff) => {
                self.push_message(TuiMessage::DiffView(diff));
            }
            HarnessMsg::TokenUsage {
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
            HarnessMsg::AskUser { .. } => {}
            HarnessMsg::TurnStarted(turn_started) => {
                self.snapshot.turn_started = Some(turn_started);
                self.editor.set_status("Waiting for agent...");
            }
            HarnessMsg::TurnDuration(duration_ms) => {
                self.push_message(TuiMessage::TurnDuration(duration_ms));
            }
            HarnessMsg::QueueChanged(count) => {
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

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.history.clear();
    }

    pub fn toggle_thinking(&mut self) {
        self.editor.set_status(self.display_prefs.toggle_thinking());
        self.save_display_prefs();
        self.history.invalidate_from(0);
    }

    pub fn toggle_token_info(&mut self) {
        self.editor
            .set_status(self.display_prefs.toggle_token_info());
        self.save_display_prefs();
        self.history.invalidate_from(0);
    }

    pub fn toggle_diff_view(&mut self) {
        self.editor
            .set_status(self.display_prefs.toggle_diff_view());
        self.save_display_prefs();
        self.history.invalidate_from(0);
    }

    pub fn save_display_prefs(&self) {
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
