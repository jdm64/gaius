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
    agents::AgentDefinition,
    session::Session,
    token_usage::{TokenUsageLedger, format_arrows},
    tools::{ToolEngine, ToolResult},
    util::prompt_input,
};
use futures::StreamExt;
use genai::{
    Client, ModelIden, ServiceTarget,
    adapter::AdapterKind,
    chat::{
        ChatMessage, ChatOptions, ChatRequest, ChatRole, ChatStreamEvent, ContentPart, ToolCall,
        ToolResponse,
    },
    resolver::{AuthData, Endpoint, ServiceTargetResolver},
};
use std::{
    error::Error,
    io::{self, Write},
};

pub fn create_client(kind: AdapterKind, url: String, key: String, model: String) -> Client {
    let resolver = ServiceTargetResolver::from_resolver_fn(
        move |mut service_target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
            service_target.endpoint = Endpoint::from_owned(url.clone());
            service_target.auth = AuthData::Key(key.clone());
            service_target.model = ModelIden::new(kind, model.clone());
            Ok(service_target)
        },
    );
    Client::builder()
        .with_service_target_resolver(resolver)
        .build()
}

pub async fn validate_model(
    client: &Client,
    model: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let request = ChatRequest::from_user("Reply with ok.");
    client.exec_chat(model, request, None).await?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub enum HarnessEvent {
    UserPrompt(String),
    AgentMessage(String),
    Thinking(String),
    ToolCall {
        name: String,
        arguments: String,
        result: String,
        error: bool,
    },
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
    pub model: String,
    pub agent_name: String,
    pub streaming: bool,
}

pub struct Harness {
    history: ChatRequest,
    client: Client,
    tool_engine: ToolEngine,
    model: String,
    agent: AgentDefinition,
    oneshot_prompt: Option<String>,
    session: Session,
    token_usage: TokenUsageLedger,
    streaming: bool,
}

impl Harness {
    pub fn new(
        client: Client,
        model: String,
        agent: AgentDefinition,
        oneshot_prompt: Option<String>,
        session_id: Option<String>,
    ) -> Result<Self, Box<dyn Error>> {
        let tool_engine = ToolEngine {};
        let session = match session_id {
            Some(id) => Session::new_named(id)?,
            None if oneshot_prompt.is_some() => Session::new_empty(),
            None => Session::new(),
        };

        let (mut history, token_usage) = session.load()?;
        history.tools = Some(tool_engine.build_tools());
        apply_agent_prompt(&mut history, &agent);

        Ok(Self {
            history,
            client,
            tool_engine,
            model,
            agent,
            oneshot_prompt,
            session,
            token_usage,
            streaming: true,
        })
    }

    pub fn is_oneshot(&self) -> bool {
        self.oneshot_prompt.is_some()
    }

    pub fn session_id(&self) -> Option<String> {
        self.session.id.clone()
    }

    pub fn model(&self) -> &String {
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

    pub fn set_model(&mut self, client: Client, model: String) {
        self.client = client;
        self.model = model;
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
        apply_agent_prompt(&mut self.history, &self.agent);
        Ok(())
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
                        on_event(HarnessEvent::UserPrompt(text));
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
                    token_usage.emit_usage(index, &mut on_event);
                }
                ChatRole::System => {
                    pending_tool_calls.clear();
                    token_usage.emit_usage(index, &mut on_event);
                }
            }
        }
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(prompt) = self.oneshot_prompt.take() {
            self.run_turn(prompt).await?;
        } else {
            loop {
                let input = prompt_input("user> ")?;
                if input.is_empty() {
                    break;
                }
                self.run_turn(input).await?;
            }
        };

        Ok(())
    }

    pub async fn run_turn(&mut self, prompt: String) -> Result<(), Box<dyn std::error::Error>> {
        let mut agent_started = false;
        self.run_turn_with_events(prompt, |event| match event {
            HarnessEvent::UserPrompt(text) => {
                println!("user> {}", text);
                let _ = io::stdout().flush();
                None
            }
            HarnessEvent::Thinking(text) => {
                if !agent_started {
                    print!("agent> ");
                    agent_started = true;
                }
                print!("{}", text);
                let _ = io::stdout().flush();
                None
            }
            HarnessEvent::AgentMessage(text) => {
                if !agent_started {
                    print!("agent> ");
                    agent_started = true;
                }
                print!("{}", text);
                let _ = io::stdout().flush();
                None
            }
            HarnessEvent::ToolCall {
                name,
                arguments,
                result,
                error,
            } => {
                if agent_started {
                    println!();
                    agent_started = false;
                }
                println!("tool-call> {} ({})", name, arguments);
                if error {
                    println!("tool-error> {}", result);
                } else {
                    println!("tool-result> {}", result);
                }
                None
            }
            HarnessEvent::TokenUsage {
                prompt,
                response,
                total,
            } => {
                if agent_started {
                    println!();
                    agent_started = false;
                }
                let net = format_arrows(prompt, response);
                let total_str = total.unwrap_or_default();
                println!("tokens> {net} {total_str}");
                None
            }
            HarnessEvent::AskUser { title, options } => {
                if agent_started {
                    println!();
                    agent_started = false;
                }
                println!("question> {}", title);
                for (index, option) in options.iter().enumerate() {
                    println!("  {}) {}", index + 1, option);
                }
                prompt_input("answer> ").ok()
            }
        })
        .await?;

        if agent_started {
            println!();
        }

        Ok(())
    }

    pub async fn run_turn_with_events<F>(
        &mut self,
        prompt: String,
        mut on_event: F,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        on_event(HarnessEvent::UserPrompt(prompt.clone()));
        self.history.messages.push(ChatMessage::user(prompt));

        loop {
            let tool_calls = if self.streaming {
                self.send_request_streaming(&mut on_event).await?
            } else {
                self.send_request_waiting(&mut on_event).await?
            };
            self.call_tools(&tool_calls, &mut on_event);
            self.save_history()?;
            if tool_calls.is_empty() {
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
            .exec_chat_stream(&self.model, self.history.clone(), Some(&chat_options))
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
            .exec_chat(&self.model, self.history.clone(), None)
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
            let result = self.tool_engine.execute(&tc.fn_name, &tc.fn_arguments);
            match result {
                ToolResult::Question(title, options) => {
                    let answer =
                        on_event(HarnessEvent::AskUser { title, options }).unwrap_or_default();
                    self.send_tool_call_event(tc, answer, false, on_event);
                }
                ToolResult::Text(text) => {
                    self.send_tool_call_event(tc, text, false, on_event);
                }
                ToolResult::Error(err) => {
                    self.send_tool_call_event(tc, err, true, on_event);
                }
            }
        }
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

    fn save_history(&mut self) -> Result<(), Box<dyn Error>> {
        self.session.save(&self.history, &self.token_usage)?;
        Ok(())
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
