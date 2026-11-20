/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{message::*, protocol_v2::ProtocolV2, reply},
    harness::HarnessMsg,
    session::Session,
};
use agent_client_protocol::{
    Client, ConnectionTo, Error, Responder,
    schema::ProtocolVersion,
    v2::schema::{Error as SchemaError, *},
};
use chrono::Utc;
use std::path::PathBuf;

pub fn reply_no_session(resp: Responder, id: SessionId) -> Result<(), Error> {
    resp.respond_with_error(SchemaError::session_not_found(id).into())
}

pub fn abs_path(path: Option<AbsolutePath>) -> Result<Option<PathBuf>, String> {
    match path {
        Some(p) if p.0.is_absolute() => Ok(Some(p.0)),
        None => Ok(None),
        _ => Err("Absolute path required".into()),
    }
}

impl From<ClientRequest> for ACPRequest {
    fn from(req: ClientRequest) -> Self {
        match req {
            ClientRequest::InitializeRequest(data) => {
                Self::Initialize(Box::new(InitializeV2 { data }))
            }
            ClientRequest::ListSessionsRequest(data) => {
                Self::ListSessions(Box::new(ListSessionsV2 { data }))
            }
            ClientRequest::NewSessionRequest(data) => {
                Self::NewSession(Box::new(NewSessionV2 { data }))
            }
            ClientRequest::ResumeSessionRequest(data) => {
                Self::ResumeSession(Box::new(ResumeSessionV2 { data }))
            }
            ClientRequest::CloseSessionRequest(data) => {
                Self::CloseSession(Box::new(CloseSessionV2 { data }))
            }
            ClientRequest::DeleteSessionRequest(data) => {
                Self::DeleteSession(Box::new(DeleteSessionV2 { data }))
            }
            ClientRequest::SetSessionConfigOptionRequest(data) => {
                Self::SetConfig(Box::new(SetSessionConfigOptionV2 { data }))
            }
            ClientRequest::PromptRequest(data) => Self::Prompt(Box::new(PromptV2 { data })),
            _ => Self::Other,
        }
    }
}

impl From<ClientNotification> for ACPNotification {
    fn from(note: ClientNotification) -> Self {
        match note {
            ClientNotification::CancelSessionNotification(_) => Self::CancelSession,
            _ => Self::Other,
        }
    }
}

pub struct InitializeV2 {
    data: Box<InitializeRequest>,
}

impl InitializeV2 {
    pub fn check_version(version: ProtocolVersion) -> Result<(), Error> {
        if version != ProtocolVersion::V2 {
            let err = SchemaError::unsupported_protocol_version(version, &[ProtocolVersion::V2]);
            Err(err.into())
        } else {
            Ok(())
        }
    }

    fn build_init() -> InitializeResponse {
        let ver = Implementation::new("gaius", env!("GIT_VERSION"));
        let msg = InitializeResponse::new(ProtocolVersion::V2, ver);
        let sess = SessionCapabilities::new().delete(SessionDeleteCapabilities::new());
        let auth = AgentAuthCapabilities::new();
        let cap = AgentCapabilities::new().session(sess).auth(auth);
        msg.capabilities(cap)
    }
}

impl Initialize for InitializeV2 {
    fn respond(self: Box<Self>, resp: Responder) -> Result<bool, Error> {
        if let Err(err) = Self::check_version(self.data.protocol_version) {
            resp.respond_with_error(err)?;
            Ok(false)
        } else {
            reply(resp, Self::build_init())?;
            Ok(true)
        }
    }
}

pub struct ListSessionsV2 {
    data: Box<ListSessionsRequest>,
}

impl ListSessions for ListSessionsV2 {
    fn has_cursor(&self) -> bool {
        self.data.cursor.is_some()
    }

    fn cwd(&self) -> Result<Option<PathBuf>, String> {
        abs_path(self.data.cwd.clone())
    }

    fn respond(self: Box<Self>, resp: Responder, sessions: Vec<Session>) -> Result<(), Error> {
        let info = sessions
            .into_iter()
            .filter_map(|session| {
                let id = session.id?;
                Some(SessionInfo::new(id, session.proj).title(session.name))
            })
            .collect();
        reply(resp, ListSessionsResponse::new(info))
    }
}

pub struct NewSessionV2 {
    data: Box<NewSessionRequest>,
}

impl NewSession for NewSessionV2 {
    fn has_mcp(&self) -> bool {
        !self.data.mcp_servers.is_empty()
    }

    fn cwd(&self) -> Result<Option<PathBuf>, String> {
        abs_path(Some(self.data.cwd.clone()))
    }

    fn respond(self: Box<Self>, resp: Responder, session_id: Option<String>) -> Result<(), Error> {
        if let Some(session_id) = session_id {
            reply(resp, NewSessionResponse::new(session_id))
        } else {
            resp.respond_with_error(Error::internal_error())
        }
    }
}

pub struct ResumeSessionV2 {
    data: Box<ResumeSessionRequest>,
}

impl ResumeSession for ResumeSessionV2 {
    fn do_replay(&self) -> Option<bool> {
        match self.data.replay_from {
            Some(ReplayFrom::Start(_)) => Some(true),
            Some(_) => None,
            None => Some(false),
        }
    }

    fn session_id(&self) -> String {
        self.data.session_id.to_string()
    }

    fn session_not_found(&self, resp: Responder) -> Result<(), Error> {
        reply_no_session(resp, self.data.session_id.clone())
    }

    fn send_replay(&self, cx: &ConnectionTo<Client>, session_id: &str, messages: &[HarnessMsg]) {
        messages.iter().for_each(|m| {
            if let Some(update) = ProtocolV2::to_update(m, session_id) {
                let _ = cx.send_notification(update);
            }
        });
    }

    fn finish(
        &self,
        resp: Responder,
        cx: &ConnectionTo<Client>,
        session_id: &str,
    ) -> Result<(), Error> {
        let complete = SessionUpdate::ReplayComplete(ReplayComplete::new());
        let update = UpdateSessionNotification::new(session_id.to_string(), complete);
        let _ = cx.send_notification(update);

        let idle = IdleStateUpdate::new().stop_reason(StopReason::EndTurn);
        let state = SessionUpdate::StateUpdate(StateUpdate::Idle(idle));
        let update = UpdateSessionNotification::new(session_id.to_string(), state);
        let _ = cx.send_notification(update);

        let mut info = SessionInfoUpdate::new().updated_at(Utc::now().to_rfc3339());
        if let Ok(title) = Session::name(session_id) {
            info = info.title(title);
        }
        let update = SessionUpdate::SessionInfoUpdate(info);
        let update = UpdateSessionNotification::new(session_id.to_string(), update);
        let _ = cx.send_notification(update);

        reply(resp, ResumeSessionResponse::new())
    }
}

pub struct CloseSessionV2 {
    data: Box<CloseSessionRequest>,
}

impl CloseSession for CloseSessionV2 {
    fn session_id(&self) -> String {
        self.data.session_id.to_string()
    }

    fn success(self: Box<Self>, resp: Responder) -> Result<(), Error> {
        reply(resp, CloseSessionResponse::new())
    }
}

pub struct DeleteSessionV2 {
    data: Box<DeleteSessionRequest>,
}

impl DeleteSession for DeleteSessionV2 {
    fn session_id(&self) -> String {
        self.data.session_id.to_string()
    }

    fn success(self: Box<Self>, resp: Responder) -> Result<(), Error> {
        reply(resp, DeleteSessionResponse::new())
    }
}

pub struct SetSessionConfigOptionV2 {
    data: Box<SetSessionConfigOptionRequest>,
}

impl SetConfig for SetSessionConfigOptionV2 {
    fn session_id(&self) -> String {
        self.data.session_id.to_string()
    }

    fn session_not_found(&self, resp: Responder) -> Result<(), Error> {
        reply_no_session(resp, self.data.session_id.clone())
    }

    fn respond(&self, resp: Responder) -> Result<(), Error> {
        // TODO implement real config options
        reply(resp, SetSessionConfigOptionResponse::new(vec![]))
    }
}

pub struct PromptV2 {
    data: Box<PromptRequest>,
}

impl Prompt for PromptV2 {
    fn session_id(&self) -> String {
        self.data.session_id.to_string()
    }

    fn session_not_found(&self, resp: Responder) -> Result<(), Error> {
        reply_no_session(resp, self.data.session_id.clone())
    }

    fn build_prompt(&self) -> Result<String, Error> {
        let mut text_blocks = Vec::new();
        let mut attachments = Vec::new();
        for block in &self.data.prompt {
            match block {
                ContentBlock::Text(text) => {
                    text_blocks.push(text.text.clone());
                }
                ContentBlock::ResourceLink(link) => match serde_json::to_string(link) {
                    Ok(link) => attachments.push(link),
                    Err(_) => return Err(Error::internal_error()),
                },
                _ => return Err(Error::invalid_params()),
            }
        }

        let mut prompt = text_blocks.join("\n\n");
        if !attachments.is_empty() {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str("Attachments:\n");
            prompt.push_str(&attachments.join("\n"));
        }
        Ok(prompt)
    }
}
