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

use std::{
    error::Error,
    io::{self, Write},
};

use crate::{agents::AgentDefinition, session::Session, tools::ToolEngine, util::prompt_input};
use futures::StreamExt;
use genai::{
    Client, ModelIden, ServiceTarget,
    adapter::AdapterKind,
    chat::{ChatMessage, ChatOptions, ChatRequest, ChatStreamEvent, ToolResponse},
    resolver::{AuthData, Endpoint, ServiceTargetResolver},
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

pub struct Harness {
    history: ChatRequest,
    client: Client,
    tool_engine: ToolEngine,
    model: String,
    agent: AgentDefinition,
    oneshot_prompt: Option<String>,
    session: Session,
}

pub enum HarnessEvent {
    AgentMessageChunk(String),
    ToolCall { name: String, arguments: String },
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

        let mut history = session.load()?;
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
        self.history = self.session.load()?;
        self.history.tools = Some(self.tool_engine.build_tools());
        apply_agent_prompt(&mut self.history, &self.agent);
        Ok(())
    }

    pub fn history(&self) -> &ChatRequest {
        &self.history
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
            HarnessEvent::AgentMessageChunk(text) => {
                if !agent_started {
                    print!("agent> ");
                    agent_started = true;
                }
                print!("{}", text);
                let _ = io::stdout().flush();
            }
            HarnessEvent::ToolCall { name, arguments } => {
                if agent_started {
                    println!();
                    agent_started = false;
                }
                println!("tool-call> {} ({})", name, arguments);
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
        F: FnMut(HarnessEvent),
    {
        self.history.messages.push(ChatMessage::user(prompt));

        loop {
            let chat_options = ChatOptions::default()
                .with_capture_content(true)
                .with_capture_tool_calls(true);
            let mut response = self
                .client
                .exec_chat_stream(&self.model, self.history.clone(), Some(&chat_options))
                .await?;

            let mut stream_end = None;
            let mut emitted_text = false;
            while let Some(event) = response.stream.next().await {
                match event? {
                    ChatStreamEvent::Chunk(chunk) if !chunk.content.is_empty() => {
                        emitted_text = true;
                        on_event(HarnessEvent::AgentMessageChunk(chunk.content));
                    }
                    ChatStreamEvent::End(end) => {
                        stream_end = Some(end);
                    }
                    ChatStreamEvent::Start
                    | ChatStreamEvent::Chunk(_)
                    | ChatStreamEvent::ReasoningChunk(_)
                    | ChatStreamEvent::ThoughtSignatureChunk(_)
                    | ChatStreamEvent::ToolCallChunk(_) => {}
                }
            }

            let stream_end = stream_end.ok_or("Chat stream ended without an end event")?;
            let content = stream_end.captured_content.unwrap_or_default();
            if !emitted_text {
                let text = content.texts().join("");
                if !text.is_empty() {
                    on_event(HarnessEvent::AgentMessageChunk(text));
                }
            }
            let tool_calls = content.tool_calls();
            self.history
                .messages
                .push(ChatMessage::assistant(content.clone()));

            if tool_calls.is_empty() {
                self.save_history()?;
                return Ok(());
            }

            for tc in tool_calls {
                let result = self.tool_engine.execute(&tc.fn_name, &tc.fn_arguments);
                self.history
                    .messages
                    .push(ToolResponse::new(&tc.call_id, result.clone()).into());

                on_event(HarnessEvent::ToolCall {
                    name: tc.fn_name.to_string(),
                    arguments: tc.fn_arguments.to_string(),
                });
            }
        }
    }

    fn save_history(&mut self) -> Result<(), Box<dyn Error>> {
        self.session.save(&self.history)?;
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
