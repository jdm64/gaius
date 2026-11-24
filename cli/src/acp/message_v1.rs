/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{engine::ACPEngine, message::*, protocol_v1::ProtocolV1, reply},
    harness::HarnessMsg,
    models::ReasoningEffort,
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

struct MsgBuilder;

impl MsgBuilder {
    pub fn models(engine: &ACPEngine) -> SessionConfigOption {
        let (selected, options) = engine.model_config();
        let options = options
            .into_iter()
            .map(|(value, name)| SessionConfigSelectOption::new(value, name))
            .collect();
        let options = SessionConfigSelectOptions::Ungrouped(options);
        let select = SessionConfigSelect::new(selected, options);
        SessionConfigOption::new(
            ConfigKey::MODEL,
            ConfigKey::MODEL_LABEL,
            SessionConfigKind::Select(select),
        )
        .category(SessionConfigOptionCategory::Model)
    }

    pub fn reasoning(engine: &ACPEngine) -> SessionConfigOption {
        let selected = engine.model_reasoning();
        let options = ReasoningEffort::all()
            .iter()
            .map(|effort| {
                SessionConfigSelectOption::new(effort.label().to_string(), effort.label())
            })
            .collect();
        let options = SessionConfigSelectOptions::Ungrouped(options);
        let select = SessionConfigSelect::new(selected, options);
        SessionConfigOption::new(
            ConfigKey::REASONING,
            ConfigKey::REASONING_LABEL,
            SessionConfigKind::Select(select),
        )
        .category(SessionConfigOptionCategory::ThoughtLevel)
    }

    pub fn config_options(engine: &ACPEngine) -> Vec<SessionConfigOption> {
        vec![Self::models(engine), Self::reasoning(engine)]
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

    fn respond(
        self: Box<Self>,
        resp: Responder,
        session_id: Option<String>,
        engine: &ACPEngine,
    ) -> Result<(), Error> {
        if let Some(session_id) = session_id {
            reply(
                resp,
                NewSessionResponse::new(session_id)
                    .config_options(MsgBuilder::config_options(engine)),
            )
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
            if let Some(update) = ProtocolV1::to_update(m, session_id, false) {
                let _ = cx.send_notification(update);
            }
        });
    }

    fn finish(
        &self,
        resp: Responder,
        _cx: &ConnectionTo<Client>,
        _session_id: &str,
        engine: &ACPEngine,
    ) -> Result<(), Error> {
        reply(
            resp,
            ResumeSessionResponse::new().config_options(MsgBuilder::config_options(engine)),
        )
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
            if let Some(update) = ProtocolV1::to_update(m, session_id, false) {
                let _ = cx.send_notification(update);
            }
        });
    }

    fn finish(
        &self,
        resp: Responder,
        _cx: &ConnectionTo<Client>,
        _session_id: &str,
        engine: &ACPEngine,
    ) -> Result<(), Error> {
        reply(
            resp,
            LoadSessionResponse::new().config_options(MsgBuilder::config_options(engine)),
        )
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

    fn entry(&self) -> (String, String) {
        let key = self.data.config_id.to_string();
        let value = match &self.data.value {
            SessionConfigOptionValue::Boolean { value } => value.to_string(),
            SessionConfigOptionValue::ValueId { value } => value.to_string(),
            _ => "".into(),
        };
        (key, value)
    }

    fn respond(&self, resp: Responder, engine: &ACPEngine) -> Result<(), Error> {
        reply(
            resp,
            SetSessionConfigOptionResponse::new(MsgBuilder::config_options(engine)),
        )
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

    fn prompt_data(&self) -> Result<PromptData, Error> {
        let mut text = String::new();
        let mut command = None;
        let mut attachments: Vec<Box<dyn PromptAttachment>> = Vec::new();
        for block in &self.data.prompt {
            match block {
                ContentBlock::Text(block) => {
                    if command.is_none() {
                        command = block.text.split(' ').next().map(str::to_owned);
                    }
                    text.push_str(&block.text)
                }
                ContentBlock::ResourceLink(link) => {
                    text.push_str(&format!("`{}`", link.name));
                    attachments.push(Box::new(PromptAttachmentV1(link.clone())));
                }
                _ => return Err(Error::invalid_params()),
            }
        }
        Ok(PromptData {
            text,
            command,
            attachments,
        })
    }
}

struct PromptAttachmentV1(ResourceLink);

impl PromptAttachment for PromptAttachmentV1 {
    fn name(&self) -> &str {
        &self.0.name
    }

    fn uri(&self) -> &str {
        &self.0.uri
    }

    fn to_json(&self, uri: &str) -> Result<String, Error> {
        let mut link = self.0.clone();
        link.uri = uri.to_string();
        serde_json::to_string(&link).map_err(|_| Error::internal_error())
    }
}
