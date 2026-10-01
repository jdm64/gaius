/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::AgentDefinition,
    cancel_handle::CancelHandle,
    client::LLMClient,
    compact::{Compact, CompactOutcome},
    diff_view::DiffView,
    history_replay,
    models::ModelDef,
    plan_hook::PlanHook,
    rate_limit::is_rate_limit_error,
    render_util::RenderUtil,
    session::Session,
    skills::{Skill, SkillRepo},
    token_usage::{SessionInfo, TokenUsageLedger},
    tools::{ToolEngine, ToolResult},
};
use futures::StreamExt;
use genai::chat::{
    ChatMessage, ChatRequest, ChatResponse, ChatStreamEvent, ContentPart, CustomPart,
    MessageContent, StreamEnd, ToolCall, ToolResponse, Usage,
};
use serde_json::json;
use std::{
    error::Error,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub enum UserRequest {
    Prompt(String),
    Skill(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum HarnessEvent {
    UserPrompt(String),
    PlanMessage(String),
    AgentMessage(String),
    SystemMessage(String),
    Thinking(String),
    CompactStart {
        start_time: u64,
    },
    CompactSummary(String),
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
    TokenUsage {
        prompt: Option<i32>,
        response: Option<i32>,
        total: Option<i32>,
        cost: Option<f64>,
    },
    AskUser {
        title: String,
        options: Vec<String>,
    },
    TurnStarted(u64),
    TurnDuration(u64),
}

#[derive(Clone, Debug, Default)]
pub struct HarnessSnapshot {
    pub session_id: Option<String>,
    pub has_history: bool,
    pub model: ModelDef,
    pub agent_name: String,
    pub streaming: bool,
    pub plan_mode_on: bool,
    pub total_cost: Option<f64>,
    pub turn_started: Option<u64>,
}

pub struct Harness {
    history: ChatRequest,
    client: LLMClient,
    tool_engine: ToolEngine,
    session: Session,
    token_usage: TokenUsageLedger,
    live_info: Arc<Mutex<SessionInfo>>,
    streaming: bool,
    cancel: CancelHandle,
    last_plan_content: Option<String>,
    plan_mode: bool,
    turn_start: Option<u64>,
}

impl Harness {
    /// Create a harness that persists turns to a session file.
    pub fn new(agent: AgentDefinition, session_id: Option<String>) -> Result<Self, Box<dyn Error>> {
        Self::new_with_session(agent, session_id, true)
    }

    /// Create a harness for one-shot prompts that should not be persisted.
    pub fn new_without_session(agent: AgentDefinition) -> Result<Self, Box<dyn Error>> {
        Self::new_with_session(agent, None, false)
    }

    fn new_with_session(
        agent: AgentDefinition,
        session_id: Option<String>,
        create_session: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let skill_repo = SkillRepo::load().unwrap_or_else(|e| {
            eprintln!("Warning: Failed to load skills: {}", e);
            SkillRepo::default()
        });
        let tool_engine = ToolEngine::new(skill_repo);
        let session = match session_id {
            Some(id) => Session::new_named(id)?,
            None if create_session => Session::new(),
            None => Session::new_empty(),
        };

        let (history, token_usage) = session.load()?;
        let live_info = Arc::new(Mutex::new(SessionInfo {
            id: session.id.clone(),
            usage: token_usage.usage(),
        }));

        let mut harness = Self {
            history,
            client: LLMClient::new(agent),
            tool_engine,
            session,
            token_usage,
            streaming: true,
            cancel: CancelHandle::new(),
            last_plan_content: None,
            plan_mode: false,
            live_info,
            turn_start: None,
        };

        harness.build_sys_prompt();

        Ok(harness)
    }

    pub fn session_id(&self) -> Option<String> {
        self.session.id.clone()
    }

    pub fn client(&self) -> &LLMClient {
        &self.client
    }

    pub fn streaming(&self) -> bool {
        self.streaming
    }

    pub fn set_streaming(&mut self, streaming: bool) {
        self.streaming = streaming;
    }

    pub fn plan_mode(&self) -> bool {
        self.plan_mode
    }

    pub fn set_plan_mode(&mut self, is_on: bool) {
        self.plan_mode = is_on;
        self.build_sys_prompt();
    }

    fn build_sys_prompt(&mut self) {
        self.history.tools = if self.plan_mode {
            Some(self.tool_engine.build_tools())
        } else {
            Some(self.tool_engine.build_tools_without_plan())
        };

        self.history.system = self
            .client
            .sys_prompt(&self.tool_engine.skill_repo, self.plan_mode)
    }

    pub fn is_cancel(&self) -> bool {
        self.cancel.is_cancel()
    }

    pub fn set_cancel(&self, val: bool) {
        if val {
            self.cancel.cancel();
        } else {
            self.cancel.reset();
        }
    }

    pub fn cancel_handle(&self) -> CancelHandle {
        self.cancel.clone()
    }

    pub async fn set_model(&mut self, model: ModelDef) -> Result<(), Box<dyn Error>> {
        self.client.set_model(model).await?;
        Ok(())
    }

    pub fn set_agent(&mut self, agent: AgentDefinition) {
        self.client.set_agent(agent);
        self.build_sys_prompt();
    }

    pub fn reload_agent(&mut self, agent: AgentDefinition) {
        self.client.reload_agent(agent);
        self.build_sys_prompt();
    }

    pub fn new_session(&mut self) -> Result<(), Box<dyn Error>> {
        self.load_session(Session::new())
    }

    pub fn fork_session(&mut self) -> Result<(), Box<dyn Error>> {
        self.session = Session::new();
        self.save_history()
    }

    pub fn load_session_by_id(&mut self, session_id: &str) -> Result<(), Box<dyn Error>> {
        self.load_session(Session::new_named(session_id.to_string())?)
    }

    pub fn load_session(&mut self, session: Session) -> Result<(), Box<dyn Error>> {
        self.session = session;
        let (history, token_usage) = self.session.load()?;
        self.history = history;
        self.token_usage = token_usage;
        self.history.tools = Some(self.tool_engine.build_tools());
        self.last_plan_content = None;
        self.update_session_info();
        self.build_sys_prompt();
        Ok(())
    }

    pub fn plan_text(&mut self) -> &mut Option<String> {
        &mut self.last_plan_content
    }

    pub fn clear_context(&mut self) {
        self.history.messages.clear();
        self.token_usage.clear_context();
        self.update_session_info();
    }

    pub fn session_info(&self) -> Arc<Mutex<SessionInfo>> {
        Arc::clone(&self.live_info)
    }

    fn update_session_info(&self) {
        *self.live_info.lock().unwrap() = SessionInfo {
            id: self.session.id.clone(),
            usage: self.token_usage.usage(),
        };
    }

    pub fn history(&self) -> &ChatRequest {
        &self.history
    }

    pub fn token_usage(&self) -> &TokenUsageLedger {
        &self.token_usage
    }

    pub fn list_skills(&self) -> Vec<Skill> {
        self.tool_engine.skill_repo.list()
    }

    pub fn reload_skills(&mut self) -> Vec<Skill> {
        if let Ok(new_repo) = SkillRepo::load() {
            self.tool_engine.skill_repo = new_repo;
            self.build_sys_prompt();
        }
        self.list_skills()
    }

    pub fn snapshot(&self) -> HarnessSnapshot {
        HarnessSnapshot {
            session_id: self.session_id(),
            has_history: !self.history().messages.is_empty(),
            model: self.client.model().clone(),
            agent_name: self.client.agent().name.to_string(),
            streaming: self.streaming(),
            plan_mode_on: self.plan_mode,
            total_cost: self.token_usage.usage().total_cost(),
            turn_started: self.turn_start,
        }
    }

    /// Replay the entire chat history as `HarnessEvent` callbacks, pairing
    /// assistant tool-calls with their following tool-response messages.
    ///
    /// TUI and CLI callers can use this as the single code path for rendering
    /// both live turns and previously-saved history.
    pub fn replay_history<F>(&self, on_event: F)
    where
        F: FnMut(HarnessEvent),
    {
        history_replay::replay_messages(&self.history.messages, &self.token_usage, on_event);
        self.update_session_info();
    }

    pub async fn compact<F>(&mut self, on_event: &mut F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        self.set_cancel(false);
        self.update_session_info();

        match Compact::compact_now(self, on_event).await? {
            CompactOutcome::NothingToCompact => {
                self.send_system_message("Nothing to compact".to_string(), on_event);
            }
            CompactOutcome::Cancelled => {
                self.send_system_message("Compaction cancelled".to_string(), on_event);
            }
            // A failure already reported itself with a system message.
            CompactOutcome::Compacted | CompactOutcome::Failed => {}
        }

        Ok(())
    }

    pub async fn run_turn<F>(
        &mut self,
        request: UserRequest,
        mut on_event: F,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let start = time_now();
        self.turn_start = Some(start);
        on_event(HarnessEvent::TurnStarted(start));
        let result = self.run_turn_with_events(request, &mut on_event).await;
        self.turn_start = None;
        let duration = time_now().saturating_sub(start);
        on_event(HarnessEvent::TurnDuration(duration));

        result
    }

    async fn run_turn_with_events<F>(
        &mut self,
        request: UserRequest,
        mut on_event: F,
    ) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        self.set_cancel(false);
        self.update_session_info();

        match request {
            UserRequest::Prompt(text) => self.send_user_message(text, &mut on_event),
            UserRequest::Skill(name) => self.send_skill_message(name, &mut on_event)?,
        }

        loop {
            if self.is_cancel() {
                self.send_system_message("Request Cancelled".to_string(), &mut on_event);
                return Ok(());
            }

            // A cancelled compaction means the user wants out of the turn —
            // sending the request right after would ignore the cancel.
            if Compact::maybe_compact(self, &mut on_event).await? == CompactOutcome::Cancelled {
                self.send_system_message("Request Cancelled".to_string(), &mut on_event);
                return Ok(());
            }

            let tool_calls = if self.streaming {
                self.send_request_streaming(&mut on_event).await?
            } else {
                self.send_request_waiting(&mut on_event).await?
            };

            self.call_tools(&tool_calls, &mut on_event).await;

            let stop_requested = PlanHook::run(self, &mut on_event);

            self.save_history()?;

            if stop_requested || tool_calls.is_empty() {
                if self.is_cancel() {
                    self.send_system_message("Request Cancelled".to_string(), &mut on_event);
                }
                return Ok(());
            }
        }
    }

    async fn send_request_streaming<F>(
        &mut self,
        on_event: &mut F,
    ) -> Result<Vec<ToolCall>, Box<dyn std::error::Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let max_retries = 4;
        let mut delay = 3u64;

        for attempt in 0..=max_retries {
            match self.try_send_request_streaming(on_event).await {
                Ok(tool_calls) => return Ok(tool_calls),
                Err(err) => {
                    let is_rate_limit = is_rate_limit_error(err.as_ref());
                    if !is_rate_limit || attempt >= max_retries {
                        return Err(err);
                    }
                    // fall through to retry below
                }
            }

            let message = format!(
                "Rate limit hit. Retrying in {delay}s (attempt {}/{max_retries})...",
                attempt + 1
            );
            on_event(HarnessEvent::SystemMessage(message));

            tokio::select! {
                _ = time::sleep(Duration::from_secs(delay)) => {}
                _ = self.cancel.notified() => {
                    if self.is_cancel() {
                        return Ok(vec![]);
                    }
                }
            }
            delay *= 3;
        }

        Err(format!("Rate limit exceeded after {} retries", max_retries,).into())
    }

    async fn try_send_request_streaming<F>(
        &mut self,
        on_event: &mut F,
    ) -> Result<Vec<ToolCall>, Box<dyn std::error::Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let prompt_idx = self.history.messages.len();
        let mut response = self.client.chat_streaming(self.history.clone()).await?;

        let mut stream_end = None;
        let mut emitted_text = false;
        loop {
            tokio::select! {
                event = response.stream.next() => {
                    match event {
                        Some(Ok(event)) => {
                            emitted_text |= Self::handle_stream_event(
                                event, on_event, &mut stream_end,
                            );
                        }
                        Some(Err(err)) => return Err(err.into()),
                        None => break,
                    }
                }
                _ = self.cancel.notified() => {
                    if self.is_cancel() {
                        return Ok(vec![]);
                    }
                }
            }
        }

        let stream_end = stream_end.ok_or("Chat stream ended without an end event")?;
        let content = stream_end.captured_content.unwrap_or_default();

        if !emitted_text {
            let text = content.texts().join("");
            if !text.is_empty() {
                on_event(HarnessEvent::AgentMessage(text));
            }
        }

        self.history.messages.push(
            ChatMessage::assistant(content.clone())
                .with_reasoning_content(stream_end.captured_reasoning_content.clone()),
        );

        let response_idx = self.history.messages.len() - 1;
        self.record_usage(
            prompt_idx,
            response_idx,
            stream_end.captured_usage,
            on_event,
        );

        Ok(content.into_tool_calls())
    }

    fn handle_stream_event<F>(
        event: ChatStreamEvent,
        on_event: &mut F,
        stream_end: &mut Option<StreamEnd>,
    ) -> bool
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        match event {
            ChatStreamEvent::Chunk(chunk) => {
                if !chunk.content.is_empty() {
                    on_event(HarnessEvent::AgentMessage(chunk.content));
                    true
                } else {
                    false
                }
            }
            ChatStreamEvent::ReasoningChunk(chunk) => {
                if !chunk.content.is_empty() {
                    on_event(HarnessEvent::Thinking(chunk.content));
                }
                false
            }
            ChatStreamEvent::ThoughtSignatureChunk(chunk) => {
                if !chunk.content.is_empty() {
                    on_event(HarnessEvent::Thinking(chunk.content));
                }
                false
            }
            ChatStreamEvent::End(end) => {
                *stream_end = Some(end);
                false
            }
            ChatStreamEvent::Start | ChatStreamEvent::ToolCallChunk(_) => false,
        }
    }

    async fn send_request_waiting<F>(
        &mut self,
        on_event: &mut F,
    ) -> Result<Vec<ToolCall>, Box<dyn std::error::Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let prompt_idx = self.history.messages.len();
        let response = self.client.chat(self.history.clone()).await?;

        let full_text = response.content.texts().join("");
        if !full_text.is_empty() {
            on_event(HarnessEvent::AgentMessage(full_text.clone()));
        }

        self.history
            .messages
            .push(ChatMessage::assistant(response.content.clone()));

        let response_idx = self.history.messages.len() - 1;
        self.record_usage(prompt_idx, response_idx, Some(response.usage), on_event);

        Ok(response.content.into_tool_calls())
    }

    pub async fn exec_chat_cancellable(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, Box<dyn Error>> {
        let response = self.client.chat(request);
        tokio::pin!(response);

        loop {
            tokio::select! {
                response = &mut response => return response,
                _ = self.cancel.notified() => {
                    // A permit can be left over from an earlier cancel that no
                    // waiter consumed; only a live cancel flag aborts.
                    if self.is_cancel() {
                        return Err("request cancelled".into());
                    }
                }
            }
        }
    }

    fn record_usage<F>(
        &mut self,
        prompt_idx: usize,
        response_idx: usize,
        usage_opt: Option<Usage>,
        on_event: &mut F,
    ) where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let Some(usage) = usage_opt else {
            return;
        };

        let pricing = self.client.model().pricing.as_ref();
        self.token_usage
            .record_with_event(prompt_idx, response_idx, &usage, pricing, on_event);
    }

    /// Record the usage of a side request — one whose messages are not part of
    /// the chat history, like the compaction summary call — so its tokens and
    /// cost are counted in the session usage.
    pub fn record_side_usage<F>(&mut self, usage: &Usage, on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let pricing = self.client.model().pricing.as_ref();
        self.token_usage.record_side_usage(usage, pricing);
        on_event(HarnessEvent::TokenUsage {
            prompt: usage.prompt_tokens,
            response: usage.completion_tokens,
            // A side request (such as compaction) does not change the active
            // conversation context. `apply_compaction` emits the new total
            // after it replaces the history.
            total: None,
            cost: self.token_usage.usage().total_cost(),
        });
        self.update_session_info();
    }

    async fn call_tools<F>(&mut self, tool_calls: &[ToolCall], on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        for tc in tool_calls {
            if self.is_cancel() {
                return;
            }

            self.send_tool_call_event(tc, on_event);

            let result = self
                .tool_engine
                .execute(&tc.fn_name, &tc.fn_arguments)
                .await;
            match result {
                ToolResult::Question(title, options) => {
                    let answer =
                        on_event(HarnessEvent::AskUser { title, options }).unwrap_or_default();
                    self.send_tool_result_event(tc, answer, false, on_event);
                }
                ToolResult::Text(text) => {
                    if tc.fn_name == "plan" {
                        let plan_text = RenderUtil::plan_to_md(&tc.fn_arguments);
                        self.last_plan_content = Some(plan_text);
                    }
                    self.send_tool_result_event(tc, text, false, on_event);
                }
                ToolResult::FileEdit { message, diff } => {
                    self.send_diff_view_event(tc, message, diff, on_event);
                }
                ToolResult::Error(err) => {
                    self.send_tool_result_event(tc, err, true, on_event);
                }
            }
        }
    }

    fn send_skill_message<F>(&mut self, name: String, mut on_event: F) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let skill_command = format!("/skill {}", name);
        self.send_user_message(skill_command, &mut on_event);

        let tc = ToolCall {
            call_id: Uuid::new_v4().to_string(),
            fn_name: "skill".to_string(),
            fn_arguments: json!({ "name": name }),
            thought_signatures: None,
        };
        let assistant_content = MessageContent::from_tool_calls(vec![tc.clone()]);
        self.history
            .messages
            .push(ChatMessage::assistant(assistant_content.clone()));

        if let Some(text) = assistant_content.joined_texts() {
            on_event(HarnessEvent::AgentMessage(text));
        }

        self.send_tool_call_event(&tc, &mut on_event);

        let result = self.tool_engine.load_skill(&name);
        match result {
            ToolResult::Text(text) => self.send_tool_result_event(&tc, text, false, &mut on_event),
            ToolResult::Error(err) => self.send_tool_result_event(&tc, err, true, &mut on_event),
            _ => {
                return Err("Tool engine failed to run skill".into());
            }
        }

        Ok(())
    }

    pub fn send_user_message<F>(&mut self, message: String, on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        self.history
            .messages
            .push(ChatMessage::user(message.clone()));
        on_event(HarnessEvent::UserPrompt(message));
    }

    pub fn send_plan_message<F>(&mut self, message: String, on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let marker = ContentPart::Custom(CustomPart {
            model_iden: None,
            data: json!("plan"),
        });
        let content = MessageContent::from_parts(vec![marker, ContentPart::Text(message.clone())]);

        self.history.messages.push(ChatMessage::user(content));
        on_event(HarnessEvent::PlanMessage(message));
    }

    fn send_system_message<F>(&self, message: String, on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        on_event(HarnessEvent::SystemMessage(message));
    }

    fn send_tool_call_event<F>(&mut self, tc: &ToolCall, on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        on_event(HarnessEvent::ToolCall {
            name: tc.fn_name.to_string(),
            arguments: tc.fn_arguments.to_string(),
            start_time: time_now(),
        });
    }

    fn send_tool_result_event<F>(
        &mut self,
        tc: &ToolCall,
        result: String,
        error: bool,
        on_event: &mut F,
    ) where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let mut message: ChatMessage = ToolResponse::new(&tc.call_id, result.clone()).into();
        message.content.push(ContentPart::Custom(CustomPart {
            model_iden: None,
            data: json!({ "tool_error": error }),
        }));
        self.history.messages.push(message);
        on_event(HarnessEvent::ToolResult {
            name: tc.fn_name.to_string(),
            result,
            error,
        });
    }

    fn send_diff_view_event<F>(
        &mut self,
        tc: &ToolCall,
        result: String,
        diff: DiffView,
        on_event: &mut F,
    ) where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let marker = diff.to_part();
        let mut message: ChatMessage = ToolResponse::new(&tc.call_id, result.clone()).into();
        message.content.push(marker);
        self.history.messages.push(message);
        on_event(HarnessEvent::ToolResult {
            name: tc.fn_name.to_string(),
            result,
            error: false,
        });
        on_event(HarnessEvent::DiffView(diff));
    }

    pub fn apply_compaction<F>(
        &mut self,
        removed: usize,
        messages: Vec<ChatMessage>,
        summary_tokens: Option<i32>,
        on_event: &mut F,
    ) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        self.history.messages = messages;
        self.token_usage.compact(removed, summary_tokens);
        self.save_history()?;
        on_event(HarnessEvent::TokenUsage {
            prompt: None,
            response: None,
            total: self.token_usage.total_tokens(),
            cost: self.token_usage.usage().total_cost(),
        });
        Ok(())
    }

    fn save_history(&mut self) -> Result<(), Box<dyn Error>> {
        self.session.save(&self.history, &self.token_usage)?;
        self.update_session_info();
        Ok(())
    }
}

pub fn time_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
