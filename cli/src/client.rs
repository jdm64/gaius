/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{agents::AgentDefinition, models::ModelDef, plan_hook::PlanHook, skills::SkillRepo};
use genai::{
    Client, Headers,
    chat::{ChatOptions, ChatRequest, ChatResponse, ChatStreamResponse},
};
use std::{error::Error, fs, path::PathBuf, sync::OnceLock};

pub struct LLMClient {
    client: Client,
    model: ModelDef,
    agent: AgentDefinition,
    agents_md: Option<String>,
}

impl LLMClient {
    pub fn new(agent: AgentDefinition) -> Self {
        Self {
            client: Client::default(),
            model: ModelDef::default(),
            agent,
            agents_md: read_agents_md(),
        }
    }

    pub fn model(&self) -> &ModelDef {
        &self.model
    }

    pub async fn set_model(&mut self, model: ModelDef) -> Result<(), Box<dyn Error>> {
        self.client = model.create_client()?;
        self.model = model;
        Ok(())
    }

    pub fn agent(&self) -> &AgentDefinition {
        &self.agent
    }

    pub fn set_agent(&mut self, agent: AgentDefinition) {
        self.agent = agent;
    }

    pub fn reload_agent(&mut self, agent: AgentDefinition) {
        self.agents_md = read_agents_md();
        self.set_agent(agent);
    }

    pub fn sys_prompt(&self, skill_repo: &SkillRepo, plan_mode: bool) -> Option<String> {
        let prompt = if plan_mode {
            format!("{}\n\n{}", self.agent.prompt, PlanHook::sys_prompt())
                .trim()
                .to_string()
        } else {
            self.agent.prompt.clone()
        };

        // Prepend AGENTS.md content if available
        let prompt = if let Some(agents_md) = &self.agents_md {
            if prompt.is_empty() {
                agents_md.to_string()
            } else {
                format!("{}\n\n{}", agents_md, prompt)
            }
        } else {
            prompt
        };

        // Append skills section to system prompt if available
        let prompt = if let Some(skills_prompt) = skill_repo.sys_prompt() {
            if prompt.is_empty() {
                skills_prompt
            } else {
                format!("{}\n\n{}", prompt, skills_prompt)
            }
        } else {
            prompt
        };

        if prompt.is_empty() {
            None
        } else {
            Some(prompt)
        }
    }

    fn get_chat_opts(&self) -> ChatOptions {
        let mut opts = base_chat_opts().clone();
        let mut headers = Headers::default();
        self.model.provider.add_headers(&mut headers);
        opts = opts.with_extra_headers(headers);
        match self.model.reasoning.as_ref().and_then(|e| e.to_genai()) {
            Some(genai_effort) => opts.with_reasoning_effort(genai_effort),
            None => opts,
        }
    }

    pub async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, Box<dyn Error>> {
        let chat_options = self.get_chat_opts();
        let response = self
            .client
            .exec_chat(&self.model.id, request, Some(&chat_options))
            .await?;
        Ok(response)
    }

    pub async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> Result<ChatStreamResponse, Box<dyn Error>> {
        let chat_options = self.get_chat_opts();
        let response = self
            .client
            .exec_chat_stream(&self.model.id, request, Some(&chat_options))
            .await?;
        Ok(response)
    }
}

static BASE_CHAT_OPTIONS: OnceLock<ChatOptions> = OnceLock::new();

fn base_chat_opts() -> &'static ChatOptions {
    BASE_CHAT_OPTIONS.get_or_init(|| {
        ChatOptions::default()
            .with_capture_content(true)
            .with_capture_tool_calls(true)
            .with_capture_reasoning_content(true)
            .with_capture_usage(true)
    })
}

fn read_agents_md() -> Option<String> {
    let path = PathBuf::from("AGENTS.md");
    if path.exists() {
        match fs::read_to_string(&path) {
            Ok(content) => {
                let content = content.trim().to_string();
                if content.is_empty() {
                    None
                } else {
                    Some(content)
                }
            }
            Err(_) => None,
        }
    } else {
        None
    }
}
