/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{message::*, protocol_v1::ProtocolV1, reply},
    harness::HarnessMsg,
    session::Session,
};
use agent_client_protocol::{
    Client, ConnectionTo, Error, Responder,
    schema::ProtocolVersion,
    v1::schema::{Error as SchemaError, *},
};
use serde_json::json;
use std::path::PathBuf;

pub fn reply_no_session(resp: Responder, id: SessionId) -> Result<(), Error> {
    resp.respond_with_error(SchemaError::session_not_found(id).into())
}

pub fn abs_path(path: Option<PathBuf>) -> Result<Option<PathBuf>, String> {
    match path {
        Some(p) if p.is_absolute() => Ok(Some(p)),
        None => Ok(None),
        _ => Err("Absolute path required".into()),
    }
}

impl From<ClientRequest> for ACPRequest {
    fn from(req: ClientRequest) -> Self {
        match req {
            ClientRequest::InitializeRequest(data) => Self::Initialize(Box::new(InitializeV1 {
                data: Box::new(data),
            })),
            ClientRequest::ListSessionsRequest(data) => {
                Self::ListSessions(Box::new(ListSessionsV1 {
                    data: Box::new(data),
                }))
            }
            ClientRequest::NewSessionRequest(data) => Self::NewSession(Box::new(NewSessionV1 {
                data: Box::new(data),
            })),
            ClientRequest::LoadSessionRequest(data) => {
                Self::ResumeSession(Box::new(LoadSessionV1 {
                    data: Box::new(data),
                }))
            }
            ClientRequest::ResumeSessionRequest(data) => {
                Self::ResumeSession(Box::new(ResumeSessionV1 {
                    data: Box::new(data),
                }))
            }
            ClientRequest::CloseSessionRequest(data) => {
                Self::CloseSession(Box::new(CloseSessionV1 {
                    data: Box::new(data),
                }))
            }
            ClientRequest::DeleteSessionRequest(data) => {
                Self::DeleteSession(Box::new(DeleteSessionV1 {
                    data: Box::new(data),
                }))
            }
            ClientRequest::SetSessionConfigOptionRequest(data) => {
                Self::SetConfig(Box::new(SetSessionConfigOptionV1 {
                    data: Box::new(data),
                }))
            }
            ClientRequest::PromptRequest(data) => Self::Prompt(Box::new(PromptV1 {
                data: Box::new(data),
            })),
            _ => Self::Other,
        }
    }
}

impl From<ClientNotification> for ACPNotification {
    fn from(note: ClientNotification) -> Self {
        match note {
            ClientNotification::CancelNotification(_) => Self::CancelSession,
            _ => Self::Other,
        }
    }
}

pub struct InitializeV1 {
    data: Box<InitializeRequest>,
}

impl InitializeV1 {
    pub fn check_version(version: ProtocolVersion) -> Result<(), Error> {
        if version == ProtocolVersion::V1 || version == ProtocolVersion::V0 {
            Ok(())
        } else {
            let mut err = SchemaError::invalid_params();
            let ver_num = version.as_u16();
            err = err.data(json!({
                "requested": ver_num,
                "supported": [1]
            }));
            Err(err.into())
        }
    }

    fn build_init() -> InitializeResponse {
        let ver = Implementation::new("gaius", env!("GIT_VERSION"));
        let msg = InitializeResponse::new(ProtocolVersion::V1).agent_info(ver);
        let sess = SessionCapabilities::new()
            .list(SessionListCapabilities::new())
            .resume(SessionResumeCapabilities::new())
            .close(SessionCloseCapabilities::new())
            .delete(SessionDeleteCapabilities::new());
        let auth = AgentAuthCapabilities::new();
        let cap = AgentCapabilities::new()
            .load_session(true)
            .session_capabilities(sess)
            .auth(auth);
        msg.agent_capabilities(cap)
    }
}

impl Initialize for InitializeV1 {
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

pub struct ListSessionsV1 {
    data: Box<ListSessionsRequest>,
}

impl ListSessions for ListSessionsV1 {
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

pub struct NewSessionV1 {
    data: Box<NewSessionRequest>,
}

impl NewSession for NewSessionV1 {
    fn has_mcp(&self) -> bool {
        false
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

pub struct ResumeSessionV1 {
    data: Box<ResumeSessionRequest>,
}

impl ResumeSession for ResumeSessionV1 {
    fn do_replay(&self) -> Option<bool> {
        Some(false)
    }

    fn session_id(&self) -> String {
        self.data.session_id.to_string()
    }

    fn session_not_found(&self, resp: Responder) -> Result<(), Error> {
        reply_no_session(resp, self.data.session_id.clone())
    }

    fn send_replay(&self, cx: &ConnectionTo<Client>, session_id: &str, messages: &[HarnessMsg]) {
        messages.iter().for_each(|m| {
            if let Some(update) = ProtocolV1::to_update(m, session_id) {
                let _ = cx.send_notification(update);
            }
        });
    }

    fn finish(
        &self,
        resp: Responder,
        _cx: &ConnectionTo<Client>,
        _session_id: &str,
    ) -> Result<(), Error> {
        reply(resp, ResumeSessionResponse::new())
    }
}

pub struct LoadSessionV1 {
    data: Box<LoadSessionRequest>,
}

impl ResumeSession for LoadSessionV1 {
    fn do_replay(&self) -> Option<bool> {
        Some(true)
    }

    fn session_id(&self) -> String {
        self.data.session_id.to_string()
    }

    fn session_not_found(&self, resp: Responder) -> Result<(), Error> {
        reply_no_session(resp, self.data.session_id.clone())
    }

    fn send_replay(&self, cx: &ConnectionTo<Client>, session_id: &str, messages: &[HarnessMsg]) {
        messages.iter().for_each(|m| {
            if let Some(update) = ProtocolV1::to_update(m, session_id) {
                let _ = cx.send_notification(update);
            }
        });
    }

    fn finish(
        &self,
        resp: Responder,
        _cx: &ConnectionTo<Client>,
        _session_id: &str,
    ) -> Result<(), Error> {
        reply(resp, LoadSessionResponse::new())
    }
}

pub struct CloseSessionV1 {
    data: Box<CloseSessionRequest>,
}

impl CloseSession for CloseSessionV1 {
    fn session_id(&self) -> String {
        self.data.session_id.to_string()
    }

    fn success(self: Box<Self>, resp: Responder) -> Result<(), Error> {
        reply(resp, CloseSessionResponse::new())
    }
}

pub struct DeleteSessionV1 {
    data: Box<DeleteSessionRequest>,
}

impl DeleteSession for DeleteSessionV1 {
    fn session_id(&self) -> String {
        self.data.session_id.to_string()
    }

    fn success(self: Box<Self>, resp: Responder) -> Result<(), Error> {
        reply(resp, DeleteSessionResponse::new())
    }
}

pub struct SetSessionConfigOptionV1 {
    data: Box<SetSessionConfigOptionRequest>,
}

impl SetConfig for SetSessionConfigOptionV1 {
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

pub struct PromptV1 {
    data: Box<PromptRequest>,
}

impl Prompt for PromptV1 {
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
