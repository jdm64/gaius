/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{
        state::{PendingPrompts, SessState},
        util::{abs_path, reply, reply_no_session},
    },
    actor::{CommandReply, HarnessActorClient, HarnessEvent},
    config::Config,
    harness::Harness,
    session::Session,
};
use agent_client_protocol::{
    Client, ConnectionTo, Error, Responder,
    schema::ProtocolVersion,
    v2::schema::{Error as SchemaError, *},
};
use chrono::Utc;
use tokio::sync::mpsc::UnboundedReceiver;

pub struct ACPSetup {
    pub engine: ACPEngine,
    pub events: UnboundedReceiver<HarnessEvent>,
}

pub struct ACPEngine {
    pub actor: HarnessActorClient,
    replies: UnboundedReceiver<CommandReply>,
    pub sess_state: SessState,
    pub pending_prompts: PendingPrompts,
}

impl ACPEngine {
    pub async fn setup() -> Result<ACPSetup, Box<dyn core::error::Error>> {
        let mut config = Config::new();
        config.load().await?;

        let harness = Harness::new(&config, None)?;
        let (actor, actor_rx) = HarnessActorClient::new(harness)?;
        let sess_state = SessState::default();
        let pending_prompts = PendingPrompts::default();

        Ok(ACPSetup {
            engine: Self {
                actor,
                replies: actor_rx.replies,
                sess_state,
                pending_prompts,
            },
            events: actor_rx.events,
        })
    }

    pub async fn handle_request(
        &mut self,
        req: ClientRequest,
        resp: Responder,
        cx: ConnectionTo<Client>,
    ) -> Result<(), Error> {
        match req {
            ClientRequest::InitializeRequest(data) => {
                if data.protocol_version != ProtocolVersion::V2 {
                    let err = SchemaError::unsupported_protocol_version(
                        data.protocol_version,
                        &[ProtocolVersion::V2],
                    );
                    resp.respond_with_error(err.into())?;
                } else {
                    reply(resp, Self::build_init())?;
                }
            }
            ClientRequest::ListSessionsRequest(data) => {
                self.handle_session_list(data, resp).await?
            }
            ClientRequest::NewSessionRequest(data) => self.handle_session_new(data, resp).await?,
            ClientRequest::ResumeSessionRequest(data) => {
                self.handle_session_resume(data, resp, cx).await?
            }
            ClientRequest::CloseSessionRequest(data) => {
                self.handle_session_close(data, resp).await?;
            }
            ClientRequest::DeleteSessionRequest(data) => {
                self.handle_session_delete(data, resp).await?;
            }
            ClientRequest::SetSessionConfigOptionRequest(data) => {
                if !self.sess_state.is_active_id(&data.session_id) {
                    reply_no_session(resp, data.session_id)?;
                } else {
                    // TODO implement real config options
                    reply(resp, SetSessionConfigOptionResponse::new(vec![]))?;
                }
            }
            ClientRequest::PromptRequest(data) => self.handle_prompt(data, resp).await?,
            _ => resp.respond_with_error(Error::method_not_found())?,
        }
        Ok(())
    }

    pub async fn handle_session_list(
        &mut self,
        data: Box<ListSessionsRequest>,
        resp: Responder,
    ) -> Result<(), Error> {
        if data.cursor.is_some() {
            resp.respond_with_error(Error::invalid_params())?;
            return Ok(());
        }
        let Ok(cwd) = abs_path(data.cwd) else {
            resp.respond_with_error(Error::invalid_params())?;
            return Ok(());
        };
        let sessions = Session::list(cwd.as_deref());
        let info = sessions
            .into_iter()
            .filter_map(|session| {
                let id = session.id?;
                Some(SessionInfo::new(id, session.proj).title(session.name))
            })
            .collect();
        reply(resp, ListSessionsResponse::new(info))
    }

    pub async fn handle_session_new(
        &mut self,
        data: Box<NewSessionRequest>,
        resp: Responder,
    ) -> Result<(), Error> {
        if !data.mcp_servers.is_empty() {
            resp.respond_with_error(Error::invalid_params())?;
            return Ok(());
        }
        let Ok(cwd) = abs_path(Some(data.cwd)) else {
            resp.respond_with_error(Error::invalid_params())?;
            return Ok(());
        };

        if self.actor.new_session(cwd).await.is_err() {
            resp.respond_with_error(Error::internal_error())?;
            return Ok(());
        }

        match self.replies.recv().await {
            Some(CommandReply::SessionCreated(Ok(snapshot))) => {
                if let Some(session_id) = snapshot.session_id {
                    self.sess_state.set(&session_id);
                    reply(resp, NewSessionResponse::new(session_id))
                } else {
                    resp.respond_with_error(Error::internal_error())
                }
            }
            Some(CommandReply::SessionCreated(Err(_))) | Some(CommandReply::Busy(_)) | None => {
                resp.respond_with_error(Error::internal_error())
            }
            Some(_) => resp.respond_with_error(Error::internal_error()),
        }
    }

    pub async fn handle_session_resume(
        &mut self,
        data: Box<ResumeSessionRequest>,
        resp: Responder,
        cx: ConnectionTo<Client>,
    ) -> Result<(), Error> {
        let do_replay = match data.replay_from {
            Some(ReplayFrom::Start(_)) => true,
            Some(_) => {
                resp.respond_with_error(Error::invalid_params())?;
                return Ok(());
            }
            None => false,
        };

        let session_id = data.session_id.to_string();
        if self.sess_state.is_unknown(&session_id) {
            reply_no_session(resp, data.session_id)?;
            return Ok(());
        }

        if self.actor.load_session(session_id.clone()).await.is_err() {
            resp.respond_with_error(Error::internal_error())?;
            return Ok(());
        }

        match self.replies.recv().await {
            Some(CommandReply::SessionLoaded(Ok(_))) => {
                self.sess_state.set(&session_id);
            }
            Some(CommandReply::SessionLoaded(Err(_))) | Some(_) | None => {
                resp.respond_with_error(Error::internal_error())?;
                return Ok(());
            }
        }

        if do_replay {
            if self.actor.replay_history().await.is_err() {
                resp.respond_with_error(Error::internal_error())?;
                return Ok(());
            }

            match self.replies.recv().await {
                Some(CommandReply::HistoryReplayed(messages)) => {
                    messages.iter().for_each(|m| {
                        if let Some(update) = m.to_update(&session_id) {
                            let _ = cx.send_notification(update);
                        }
                    });
                }
                Some(CommandReply::Busy(_)) | Some(_) | None => {
                    resp.respond_with_error(Error::internal_error())?;
                    return Ok(());
                }
            }
        }

        let complete = SessionUpdate::ReplayComplete(ReplayComplete::new());
        let update = UpdateSessionNotification::new(session_id.clone(), complete);
        let _ = cx.send_notification(update);

        let idle = IdleStateUpdate::new().stop_reason(StopReason::EndTurn);
        let state = SessionUpdate::StateUpdate(StateUpdate::Idle(idle));
        let update = UpdateSessionNotification::new(session_id.clone(), state);
        let _ = cx.send_notification(update);

        let mut info = SessionInfoUpdate::new().updated_at(Utc::now().to_rfc3339());
        if let Ok(title) = Session::name(&session_id) {
            info = info.title(title);
        }
        let update = SessionUpdate::SessionInfoUpdate(info);
        let update = UpdateSessionNotification::new(session_id, update);
        let _ = cx.send_notification(update);

        reply(resp, ResumeSessionResponse::new())
    }

    pub async fn handle_session_delete(
        &mut self,
        data: Box<DeleteSessionRequest>,
        resp: Responder,
    ) -> Result<(), Error> {
        let session_id = data.session_id.to_string();
        if self.sess_state.is_active(&session_id) {
            if self.actor.close_session().await.is_err() {
                resp.respond_with_error(Error::internal_error())?;
                return Ok(());
            }
            let closed = match self.replies.recv().await {
                Some(CommandReply::SessionClosed(result)) => result.is_ok(),
                Some(_) | None => false,
            };
            if !closed {
                resp.respond_with_error(Error::internal_error())?;
                return Ok(());
            }
            self.sess_state.clear();
            self.pending_prompts.cancel();
        }

        if Session::delete(&session_id).is_err() {
            resp.respond_with_error(Error::internal_error())?;
            return Ok(());
        }

        self.sess_state.remove(&session_id);
        reply(resp, DeleteSessionResponse::new())
    }

    pub async fn handle_session_close(
        &mut self,
        data: Box<CloseSessionRequest>,
        resp: Responder,
    ) -> Result<(), Error> {
        if !self.sess_state.is_active_id(&data.session_id) {
            reply(resp, CloseSessionResponse::new())?;
            return Ok(());
        }
        if self.actor.close_session().await.is_err() {
            resp.respond_with_error(Error::internal_error())?;
            return Ok(());
        }
        let closed = match self.replies.recv().await {
            Some(CommandReply::SessionClosed(result)) => result.is_ok(),
            Some(_) | None => false,
        };
        if closed {
            self.sess_state.clear();
            self.pending_prompts.cancel();
            reply(resp, CloseSessionResponse::new())
        } else {
            resp.respond_with_error(Error::internal_error())
        }
    }

    pub async fn handle_prompt(
        &mut self,
        data: Box<PromptRequest>,
        resp: Responder,
    ) -> Result<(), Error> {
        if !self.sess_state.is_active_id(&data.session_id) {
            reply_no_session(resp, data.session_id)?;
            return Ok(());
        }

        let mut text_blocks = Vec::new();
        let mut attachments = Vec::new();
        for block in data.prompt {
            match block {
                ContentBlock::Text(text) => {
                    text_blocks.push(text.text);
                }
                ContentBlock::ResourceLink(link) => match serde_json::to_string(&link) {
                    Ok(link) => attachments.push(link),
                    Err(_) => {
                        resp.respond_with_error(Error::internal_error())?;
                        return Ok(());
                    }
                },
                _ => {
                    resp.respond_with_error(Error::invalid_params())?;
                    return Ok(());
                }
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

        if self.actor.run_prompt(prompt).await.is_err() {
            return resp.respond_with_error(Error::internal_error());
        }

        match self.replies.recv().await {
            Some(CommandReply::PromptQueued { msg_id, .. }) => {
                self.pending_prompts.insert(msg_id, resp);
                Ok(())
            }
            _ => resp.respond_with_error(Error::internal_error()),
        }
    }

    pub async fn handle_notification(&self, note: ClientNotification, _cx: ConnectionTo<Client>) {
        match note {
            ClientNotification::CancelSessionNotification(_) => {
                self.pending_prompts.cancel();
                let _ = self.actor.cancel().await;
            }
            _ => { /* Ignore */ }
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
