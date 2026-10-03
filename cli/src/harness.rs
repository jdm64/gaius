/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

pub mod compact;
pub mod history_replay;
pub mod messages;
pub mod prompt_queue;
pub mod tools;
pub mod turn;

use crate::{
    agents::AgentDefinition,
    cancel_handle::CancelHandle,
    client::LLMClient,
    diff_view::DiffView,
    harness::prompt_queue::PromptQueue,
    models::ModelDef,
    session::Session,
    skills::{Skill, SkillRepo},
    token_usage::{SessionInfo, TokenUsageLedger},
    tool_engine::ToolEngine,
};
use genai::chat::{ChatRequest, ChatResponse};
use std::{
    error::Error,
    sync::{Arc, Mutex},
};

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
    QueueChanged(usize),
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
    pub queued_prompts: usize,
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
    prompt_queue: PromptQueue,
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
            prompt_queue: PromptQueue::new(),
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

    pub fn prompt_queue(&self) -> PromptQueue {
        self.prompt_queue.clone()
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

    pub fn update_session_info(&self) {
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
            queued_prompts: self.prompt_queue.len(),
        }
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

    pub fn save_history(&mut self) -> Result<(), Box<dyn Error>> {
        self.session.save(&self.history, &self.token_usage)?;
        self.update_session_info();
        Ok(())
    }
}
