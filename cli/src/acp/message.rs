/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{acp::engine::ACPEngine, harness::HarnessMsg, session::Session};
use agent_client_protocol::{Client, ConnectionTo, Error, Responder};
use std::path::{Path, PathBuf};
use url::Url;

pub struct ConfigKey;

impl ConfigKey {
    pub const MODEL: &'static str = "model";
    pub const MODEL_LABEL: &'static str = "Model";
    pub const REASONING: &'static str = "reasoning";
    pub const REASONING_LABEL: &'static str = "Reasoning";
}

pub enum ACPRequest {
    Initialize(Box<dyn Initialize>),
    ListSessions(Box<dyn ListSessions>),
    NewSession(Box<dyn NewSession>),
    ResumeSession(Box<dyn ResumeSession>),
    CloseSession(Box<dyn CloseSession>),
    DeleteSession(Box<dyn DeleteSession>),
    SetConfig(Box<dyn SetConfig>),
    Prompt(Box<dyn Prompt>),
    Other,
}

pub enum ACPNotification {
    CancelSession,
    Other,
}

pub trait Initialize: Send {
    fn respond(self: Box<Self>, resp: Responder) -> Result<bool, Error>;
}

pub trait ListSessions: Send {
    fn has_cursor(&self) -> bool;
    fn cwd(&self) -> Result<Option<PathBuf>, String>;
    fn respond(self: Box<Self>, resp: Responder, sessions: Vec<Session>) -> Result<(), Error>;
}

pub trait NewSession: Send {
    fn has_mcp(&self) -> bool;
    fn cwd(&self) -> Result<Option<PathBuf>, String>;
    fn respond(
        self: Box<Self>,
        resp: Responder,
        session_id: Option<String>,
        engine: &ACPEngine,
    ) -> Result<(), Error>;
}

pub trait ResumeSession: Send {
    fn do_replay(&self) -> Option<bool>;
    fn session_id(&self) -> String;
    fn session_not_found(&self, resp: Responder) -> Result<(), Error>;
    fn send_replay(&self, cx: &ConnectionTo<Client>, session_id: &str, messages: &[HarnessMsg]);
    fn finish(
        &self,
        resp: Responder,
        cx: &ConnectionTo<Client>,
        session_id: &str,
        engine: &ACPEngine,
    ) -> Result<(), Error>;
}

pub trait CloseSession: Send {
    fn session_id(&self) -> String;
    fn success(self: Box<Self>, resp: Responder) -> Result<(), Error>;
}

pub trait DeleteSession: Send {
    fn session_id(&self) -> String;
    fn success(self: Box<Self>, resp: Responder) -> Result<(), Error>;
}

pub trait SetConfig: Send {
    fn session_id(&self) -> String;
    fn session_not_found(&self, resp: Responder) -> Result<(), Error>;
    fn entry(&self) -> (String, String);
    fn respond(&self, resp: Responder, engine: &ACPEngine) -> Result<(), Error>;
}

pub trait Prompt: Send {
    fn session_id(&self) -> String;
    fn session_not_found(&self, resp: Responder) -> Result<(), Error>;
    fn prompt_data(&self) -> Result<PromptData, Error>;
}

pub struct PromptData {
    pub text: String,
    pub attachments: Vec<Box<dyn PromptAttachment>>,
}

pub trait PromptAttachment: Send {
    fn name(&self) -> &str;
    fn uri(&self) -> &str;
    fn to_json(&self, uri: &str) -> Result<String, Error>;
}

impl PromptData {
    pub fn build(self, workspace: Option<&Path>) -> Result<String, Error> {
        let mut prompt = self.text;
        if self.attachments.is_empty() {
            return Ok(prompt);
        }

        if !prompt.is_empty() {
            prompt.push_str("\n\n");
        }
        prompt.push_str("---\n**Attachments**\n");
        let mut attachments = Vec::with_capacity(self.attachments.len());
        for attachment in self.attachments {
            let uri = normalize_uri(attachment.uri(), workspace);
            attachments.push(attachment.to_json(&uri)?);
        }
        prompt.push_str("- ");
        prompt.push_str(&attachments.join("\n- "));
        Ok(prompt)
    }
}

fn normalize_uri(uri: &str, workspace: Option<&Path>) -> String {
    let Some(workspace) = workspace else {
        return uri.to_string();
    };
    let path = uri.strip_prefix("file://");
    let Some(path) = path else {
        return uri.to_string();
    };
    let Ok(path) = Url::parse(&format!("file://{path}")) else {
        return uri.to_string();
    };
    let Ok(path) = path.to_file_path() else {
        return uri.to_string();
    };
    match path.strip_prefix(workspace) {
        Ok(relative) => relative.to_string_lossy().into_owned(),
        Err(_) => uri.to_string(),
    }
}
