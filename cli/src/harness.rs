/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::AgentDefinition,
    config::Config,
    diff_view::DiffView,
    models::ModelDef,
    plan_hook::PlanHook,
    render::Render,
    session::Session,
    token_usage::{SessionInfo, TokenUsageLedger},
    tools::{ToolEngine, ToolResult},
};
use futures::StreamExt;
use genai::{
    Client,
    chat::{
        ChatMessage, ChatOptions, ChatRequest, ChatRole, ChatStreamEvent, ContentPart, CustomPart,
        MessageContent, ToolCall, ToolResponse,
    },
};
use serde_json::json;
use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Debug, PartialEq)]
pub enum HarnessEvent {
    UserPrompt(String),
    PlanMessage(String),
    AgentMessage(String),
    SystemMessage(String),
    Thinking(String),
    ToolCall {
        name: String,
        arguments: String,
        result: String,
        error: bool,
    },
    DiffView(DiffView),
    TokenUsage {
        prompt: Option<i32>,
        response: Option<i32>,
        total: Option<i32>,
    },
    AskUser {
        title: String,
        options: Vec<String>,
    },
}

#[derive(Clone, Debug)]
pub struct HarnessSnapshot {
    pub session_id: Option<String>,
    pub has_history: bool,
    pub model: ModelDef,
    pub agent_name: String,
    pub streaming: bool,
    pub plan_mode_on: bool,
}

pub struct Harness {
    history: ChatRequest,
    client: Client,
    tool_engine: ToolEngine,
    model: ModelDef,
    agent: AgentDefinition,
    session: Session,
    token_usage: TokenUsageLedger,
    streaming: bool,
    canceled: Arc<AtomicBool>,
    last_plan_content: Option<String>,
    plan_mode_on: bool,
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
        let tool_engine = ToolEngine {};
        let session = match session_id {
            Some(id) => Session::new_named(id)?,
            None if create_session => Session::new(),
            None => Session::new_empty(),
        };

        let (mut history, token_usage) = session.load()?;
        history.tools = Some(tool_engine.build_tools_without_plan());
        apply_agent_prompt(&mut history, &agent);

        Ok(Self {
            history,
            client: Client::default(),
            tool_engine,
            model: ModelDef::default(),
            agent,
            session,
            token_usage,
            streaming: true,
            canceled: Arc::new(AtomicBool::new(false)),
            last_plan_content: None,
            plan_mode_on: false,
        })
    }

    pub fn session_id(&self) -> Option<String> {
        self.session.id.clone()
    }

    pub fn model(&self) -> &ModelDef {
        &self.model
    }

    pub fn agent_name(&self) -> &str {
        &self.agent.name
    }

    pub fn streaming(&self) -> bool {
        self.streaming
    }

    pub fn set_streaming(&mut self, streaming: bool) {
        self.streaming = streaming;
    }

    pub fn plan_mode(&self) -> bool {
        self.plan_mode_on
    }

    pub fn set_plan_mode(&mut self, is_on: bool) {
        self.plan_mode_on = is_on;
        self.rebuild_agent();
    }

    fn rebuild_agent(&mut self) {
        self.history.tools = if self.plan_mode_on {
            Some(self.tool_engine.build_tools())
        } else {
            Some(self.tool_engine.build_tools_without_plan())
        };

        let prompt = if self.plan_mode_on {
            format!("{}\n\n{}", self.agent.prompt, PlanHook::sys_prompt())
                .trim()
                .to_string()
        } else {
            self.agent.prompt.clone()
        };

        self.history.system = if prompt.is_empty() {
            None
        } else {
            Some(prompt)
        }
    }

    fn is_cancel(&self) -> bool {
        self.canceled.load(Ordering::Relaxed)
    }

    pub fn set_cancel(&self, val: bool) {
        self.canceled.store(val, Ordering::Relaxed);
    }

    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        self.canceled.clone()
    }

    pub async fn set_model(&mut self, model: ModelDef) -> Result<(), Box<dyn Error>> {
        let mut config = Config::new();
        config.load().await?;

        let client = model.create_client(&config)?;
        self.client = client;
        self.model = model;
        Ok(())
    }

    pub fn set_agent(&mut self, agent: AgentDefinition) {
        apply_agent_prompt(&mut self.history, &agent);
        self.agent = agent;
    }

    pub fn new_session(&mut self) -> Result<(), Box<dyn Error>> {
        self.load_session(Session::new())
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
        apply_agent_prompt(&mut self.history, &self.agent);
        Ok(())
    }

    pub fn plan_text(&mut self) -> &mut Option<String> {
        &mut self.last_plan_content
    }

    pub fn clear_context(&mut self) {
        self.history.messages.clear();
        self.token_usage.clear_context();
    }

    pub fn session_info(&self) -> SessionInfo {
        SessionInfo {
            id: self.session.id.clone(),
            usage: self.token_usage.usage(),
        }
    }

    pub fn history(&self) -> &ChatRequest {
        &self.history
    }

    pub fn token_usage(&self) -> &TokenUsageLedger {
        &self.token_usage
    }

    pub fn snapshot(&self) -> HarnessSnapshot {
        HarnessSnapshot {
            session_id: self.session_id(),
            has_history: !self.history().messages.is_empty(),
            model: self.model().clone(),
            agent_name: self.agent_name().to_string(),
            streaming: self.streaming(),
            plan_mode_on: self.plan_mode_on,
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
        Self::replay_messages(&self.history.messages, &self.token_usage, on_event);
    }

    pub fn replay_messages<F>(
        history: &[ChatMessage],
        token_usage: &TokenUsageLedger,
        mut on_event: F,
    ) where
        F: FnMut(HarnessEvent),
    {
        let mut pending_tool_calls: Vec<(String, String)> = Vec::new();
        let mut messages = history.iter().enumerate().peekable();

        while let Some((index, message)) = messages.next() {
            match message.role {
                ChatRole::User => {
                    pending_tool_calls.clear();
                    let text = message.content.texts().join("");
                    if !text.is_empty() {
                        if has_plan_marker(message) {
                            on_event(HarnessEvent::PlanMessage(text));
                        } else {
                            on_event(HarnessEvent::UserPrompt(text));
                        }
                    }
                    token_usage.emit_usage(index, &mut on_event);
                }
                ChatRole::Assistant => {
                    let text = message.content.texts().join("");

                    // Emit any stored thinking/reasoning content first
                    for part in message.content.parts() {
                        match part {
                            ContentPart::ThoughtSignature(text)
                            | ContentPart::ReasoningContent(text)
                                if !text.is_empty() =>
                            {
                                on_event(HarnessEvent::Thinking(text.clone()));
                            }
                            _ => {}
                        }
                    }

                    // Collect pending tool calls from this assistant turn
                    for tc in message.content.tool_calls() {
                        pending_tool_calls.push((tc.fn_name.clone(), tc.fn_arguments.to_string()));
                    }

                    if !text.is_empty() {
                        on_event(HarnessEvent::AgentMessage(text));
                    }
                    token_usage.emit_usage(index, &mut on_event);

                    // Match consecutive Tool-role response messages to the pending
                    // tool calls in order.
                    loop {
                        let is_tool = match messages.peek() {
                            Some((_, m)) => m.role == ChatRole::Tool,
                            None => false,
                        };
                        if !is_tool {
                            break;
                        }
                        let (next_index, next_msg) = messages.next().unwrap();
                        let responses: Vec<&genai::chat::ToolResponse> =
                            next_msg.content.tool_responses();
                        for resp in responses {
                            if let Some((name, args)) = pending_tool_calls.first() {
                                on_event(HarnessEvent::ToolCall {
                                    name: (*name).clone(),
                                    arguments: (*args).clone(),
                                    result: resp.content.clone(),
                                    error: false,
                                });
                                pending_tool_calls.remove(0);
                            }
                        }
                        emit_diff_markers(next_msg, &mut on_event);
                        token_usage.emit_usage(next_index, &mut on_event);
                    }

                    // Any remaining unmatched calls — emit with empty result so
                    // the UI always renders something.
                    for (name, args) in pending_tool_calls.drain(..) {
                        on_event(HarnessEvent::ToolCall {
                            name,
                            arguments: args,
                            result: String::new(),
                            error: false,
                        });
                    }
                }
                ChatRole::Tool => {
                    // Unmatched tool response — display inline as agent text.
                    let text = message.content.texts().join("");
                    if !text.is_empty() {
                        on_event(HarnessEvent::AgentMessage(text));
                    }
                    for tr in message.content.tool_responses() {
                        on_event(HarnessEvent::AgentMessage(format!(
                            "[tool {}]: {}",
                            tr.call_id, tr.content
                        )));
                    }
                    emit_diff_markers(message, &mut on_event);
                    token_usage.emit_usage(index, &mut on_event);
                }
                ChatRole::System => {
                    pending_tool_calls.clear();
                    token_usage.emit_usage(index, &mut on_event);
                }
            }
        }
    }

    pub async fn run_turn_with_events<F>(
        &mut self,
        prompt: String,
        mut on_event: F,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        self.set_cancel(false);
        self.send_user_message(prompt, &mut on_event);

        loop {
            if self.is_cancel() {
                self.send_system_message("Request Cancelled".to_string(), &mut on_event);
                return Ok(());
            }

            let tool_calls = if self.streaming {
                self.send_request_streaming(&mut on_event).await?
            } else {
                self.send_request_waiting(&mut on_event).await?
            };

            self.call_tools(&tool_calls, &mut on_event);

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
        let prompt_message_end = self.history.messages.len();
        let chat_options = ChatOptions::default()
            .with_capture_content(true)
            .with_capture_tool_calls(true)
            .with_capture_reasoning_content(true)
            .with_capture_usage(true)
            .with_extra_headers(vec![("X-Stream-Options", "include_usage=true")]);
        let mut response = self
            .client
            .exec_chat_stream(&self.model.id, self.history.clone(), Some(&chat_options))
            .await?;

        let mut stream_end = None;
        let mut emitted_text = false;
        while let Some(event) = response.stream.next().await {
            match event? {
                ChatStreamEvent::Chunk(chunk) => {
                    if !chunk.content.is_empty() {
                        emitted_text = true;
                        on_event(HarnessEvent::AgentMessage(chunk.content));
                    }
                }
                ChatStreamEvent::ReasoningChunk(chunk) => {
                    if !chunk.content.is_empty() {
                        on_event(HarnessEvent::Thinking(chunk.content));
                    }
                }
                ChatStreamEvent::ThoughtSignatureChunk(chunk) => {
                    if !chunk.content.is_empty() {
                        on_event(HarnessEvent::Thinking(chunk.content));
                    }
                }
                ChatStreamEvent::End(end) => {
                    stream_end = Some(end);
                }
                ChatStreamEvent::Start | ChatStreamEvent::ToolCallChunk(_) => {}
            }

            if self.is_cancel() {
                return Ok(vec![]);
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

        let assistant_message_index = self.history.messages.len() - 1;
        if let Some(usage) = stream_end.captured_usage.as_ref() {
            self.token_usage.record_with_event(
                prompt_message_end,
                assistant_message_index,
                usage,
                on_event,
            );
        }

        Ok(content.into_tool_calls())
    }

    async fn send_request_waiting<F>(
        &mut self,
        on_event: &mut F,
    ) -> Result<Vec<ToolCall>, Box<dyn std::error::Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let prompt_message_end = self.history.messages.len();
        let response = self
            .client
            .exec_chat(&self.model.id, self.history.clone(), None)
            .await?;

        let full_text = response.content.texts().join("");
        if !full_text.is_empty() {
            on_event(HarnessEvent::AgentMessage(full_text.clone()));
        }

        self.history
            .messages
            .push(ChatMessage::assistant(response.content.clone()));

        let assistant_message_index = self.history.messages.len() - 1;
        self.token_usage.record_with_event(
            prompt_message_end,
            assistant_message_index,
            &response.usage,
            on_event,
        );

        Ok(response.content.into_tool_calls())
    }

    fn call_tools<F>(&mut self, tool_calls: &[ToolCall], on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        for tc in tool_calls {
            if self.is_cancel() {
                return;
            }
            let result = self.tool_engine.execute(&tc.fn_name, &tc.fn_arguments);
            match result {
                ToolResult::Question(title, options) => {
                    let answer =
                        on_event(HarnessEvent::AskUser { title, options }).unwrap_or_default();
                    self.send_tool_call_event(tc, answer, false, on_event);
                }
                ToolResult::Text(text) => {
                    if tc.fn_name == "plan" {
                        let plan_text = Render::plan_to_md(&tc.fn_arguments);
                        self.last_plan_content = Some(plan_text);
                    }
                    self.send_tool_call_event(tc, text, false, on_event);
                }
                ToolResult::FileEdit { message, diff } => {
                    self.send_diff_view_event(tc, message, diff, on_event);
                }
                ToolResult::Error(err) => {
                    self.send_tool_call_event(tc, err, true, on_event);
                }
            }
        }
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

    fn send_tool_call_event<F>(
        &mut self,
        tc: &ToolCall,
        result: String,
        error: bool,
        on_event: &mut F,
    ) where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        self.history
            .messages
            .push(ToolResponse::new(&tc.call_id, result.clone()).into());
        on_event(HarnessEvent::ToolCall {
            name: tc.fn_name.to_string(),
            arguments: tc.fn_arguments.to_string(),
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
        on_event(HarnessEvent::ToolCall {
            name: tc.fn_name.to_string(),
            arguments: tc.fn_arguments.to_string(),
            result,
            error: false,
        });
        on_event(HarnessEvent::DiffView(diff));
    }

    fn save_history(&mut self) -> Result<(), Box<dyn Error>> {
        self.session.save(&self.history, &self.token_usage)?;
        Ok(())
    }
}

fn has_plan_marker(message: &ChatMessage) -> bool {
    matches!(
        message.content.parts().first(),
        Some(ContentPart::Custom(CustomPart { data, .. })) if data == &json!("plan")
    )
}

fn emit_diff_markers<F>(message: &ChatMessage, on_event: &mut F)
where
    F: FnMut(HarnessEvent),
{
    for part in message.content.custom_parts() {
        if let Some(diff) = DiffView::from_marker(&part.data) {
            on_event(HarnessEvent::DiffView(diff));
        }
    }
}

fn apply_agent_prompt(history: &mut ChatRequest, agent: &AgentDefinition) {
    let prompt = agent.prompt.trim();
    history.system = if prompt.is_empty() {
        None
    } else {
        Some(agent.prompt.clone())
    };
}
