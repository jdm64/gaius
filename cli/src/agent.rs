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

use std::error::Error;

use crate::{session::Session, tools::ToolEngine, util::prompt_input};
use genai::{
    Client, ModelIden, ServiceTarget,
    adapter::AdapterKind,
    chat::{ChatMessage, ChatRequest, ToolResponse},
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

pub struct LLMAgent {
    history: ChatRequest,
    client: Client,
    tool_engine: ToolEngine,
    model: String,
    oneshot_prompt: Option<String>,
    session: Session,
}

pub enum AgentEvent {
    AgentMessage(String),
    ToolCall { name: String, arguments: String },
}

impl LLMAgent {
    pub fn new(
        client: Client,
        model: String,
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

        Ok(Self {
            history,
            client,
            tool_engine,
            model,
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

    pub fn set_model(&mut self, client: Client, model: String) {
        self.client = client;
        self.model = model;
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
        self.run_turn_with_events(prompt, |event| match event {
            AgentEvent::AgentMessage(text) => println!("agent> {}", text),
            AgentEvent::ToolCall { name, arguments } => {
                println!("tool-call> {} ({})", name, arguments);
            }
        })
        .await
    }

    pub async fn run_turn_with_events<F>(
        &mut self,
        prompt: String,
        mut on_event: F,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        F: FnMut(AgentEvent),
    {
        self.history.messages.push(ChatMessage::user(prompt));

        loop {
            let response = self
                .client
                .exec_chat(&self.model, self.history.clone(), None)
                .await?;

            self.history
                .messages
                .push(ChatMessage::assistant(response.content.clone()));

            let text = response.first_text().unwrap_or("").to_string();
            if !text.is_empty() {
                on_event(AgentEvent::AgentMessage(text));
            }

            let tool_calls = response.tool_calls();
            if tool_calls.is_empty() {
                self.save_history()?;
                return Ok(());
            }

            for tc in tool_calls {
                let result = self.tool_engine.execute(&tc.fn_name, &tc.fn_arguments);
                self.history
                    .messages
                    .push(ToolResponse::new(&tc.call_id, result.clone()).into());

                on_event(AgentEvent::ToolCall {
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
