/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{acp::engine::ACPEngine, harness::HarnessMsg, session::Session};
use agent_client_protocol::{Client, ConnectionTo, Error, Responder};
use std::path::PathBuf;

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
    fn build_prompt(&self) -> Result<String, Error>;
}
