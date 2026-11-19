/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{
        message::*,
        protocol::ACPProtocol,
        state::{PendingPrompts, SessState},
    },
    actor::{CommandReply, HarnessActorClient},
    session::Session,
};
use agent_client_protocol::{Client, ConnectionTo, Error, Responder};
use tokio::sync::mpsc::UnboundedReceiver;

pub struct ACPEngine {
    pub actor: HarnessActorClient,
    pub replies: UnboundedReceiver<CommandReply>,
    pub sess_state: SessState,
    pub prompts: PendingPrompts,
    pub initialized: bool,
    pub protocol: Box<dyn ACPProtocol>,
}

impl ACPEngine {
    fn cancel_prompts(&self) {
        self.prompts.cancel(self.protocol.as_ref());
    }

    pub async fn handle_request(
        &mut self,
        req: ACPRequest,
        resp: Responder,
        cx: ConnectionTo<Client>,
    ) -> Result<(), Error> {
        if !self.initialized && !matches!(&req, ACPRequest::Initialize(_)) {
            resp.respond_with_error(Error::invalid_request())?;
            return Ok(());
        }

        match req {
            ACPRequest::Initialize(data) => {
                data.respond(resp)?;
                self.initialized = true;
            }
            ACPRequest::ListSessions(data) => self.handle_session_list(data, resp).await?,
            ACPRequest::NewSession(data) => self.handle_session_new(data, resp).await?,
            ACPRequest::ResumeSession(data) => self.handle_session_resume(data, resp, cx).await?,
            ACPRequest::CloseSession(data) => self.handle_session_close(data, resp).await?,
            ACPRequest::DeleteSession(data) => self.handle_session_delete(data, resp).await?,
            ACPRequest::SetConfig(data) => {
                if !self.sess_state.is_active(&data.session_id()) {
                    data.session_not_found(resp)?;
                } else {
                    data.respond(resp)?;
                }
            }
            ACPRequest::Prompt(data) => self.handle_prompt(data, resp).await?,
            _ => resp.respond_with_error(Error::method_not_found())?,
        }
        Ok(())
    }

    pub async fn handle_session_list(
        &mut self,
        data: Box<dyn ListSessions>,
        resp: Responder,
    ) -> Result<(), Error> {
        if data.has_cursor() {
            resp.respond_with_error(Error::invalid_params())?;
            return Ok(());
        }
        let Ok(cwd) = data.cwd() else {
            resp.respond_with_error(Error::invalid_params())?;
            return Ok(());
        };
        let sessions = Session::list(cwd.as_deref());
        data.respond(resp, sessions)
    }

    pub async fn handle_session_new(
        &mut self,
        data: Box<dyn NewSession>,
        resp: Responder,
    ) -> Result<(), Error> {
        if data.has_mcp() {
            resp.respond_with_error(Error::invalid_params())?;
            return Ok(());
        }
        let Ok(cwd) = data.cwd() else {
            resp.respond_with_error(Error::invalid_params())?;
            return Ok(());
        };

        if self.actor.new_session(cwd).await.is_err() {
            resp.respond_with_error(Error::internal_error())?;
            return Ok(());
        }

        match self.replies.recv().await {
            Some(CommandReply::SessionCreated(Ok(snapshot))) => {
                let session_id = snapshot.session_id.clone();
                if let Some(id) = &session_id {
                    self.sess_state.set(id);
                }
                data.respond(resp, session_id)
            }
            Some(CommandReply::SessionCreated(Err(_))) | Some(CommandReply::Busy(_)) | None => {
                resp.respond_with_error(Error::internal_error())
            }
            Some(_) => resp.respond_with_error(Error::internal_error()),
        }
    }

    pub async fn handle_session_resume(
        &mut self,
        data: Box<dyn ResumeSession>,
        resp: Responder,
        cx: ConnectionTo<Client>,
    ) -> Result<(), Error> {
        let Some(do_replay) = data.do_replay() else {
            resp.respond_with_error(Error::invalid_params())?;
            return Ok(());
        };

        let session_id = data.session_id();
        if self.sess_state.is_unknown(&session_id) {
            return data.session_not_found(resp);
        }

        if self.actor.load_session(session_id.clone()).await.is_err() {
            resp.respond_with_error(Error::internal_error())?;
            return Ok(());
        }

        match self.replies.recv().await {
            Some(CommandReply::SessionLoaded(Ok(_))) => {
                self.sess_state.set(&session_id);
            }
            err => {
                // TODO V1 fails with this:
                // Some(Busy(\"Agent is busy; wait for idle before running this command\"))"}
                resp.respond_with_error(Error::internal_error().data(format!("{:?}", err)))?;
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
                    data.send_replay(&cx, &session_id, &messages);
                }
                Some(CommandReply::Busy(_)) | Some(_) | None => {
                    resp.respond_with_error(Error::internal_error())?;
                    return Ok(());
                }
            }
        }

        data.finish(resp, &cx, &session_id)
    }

    pub async fn handle_session_delete(
        &mut self,
        data: Box<dyn DeleteSession>,
        resp: Responder,
    ) -> Result<(), Error> {
        let session_id = data.session_id();
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
            self.cancel_prompts();
        }

        if Session::delete(&session_id).is_err() {
            resp.respond_with_error(Error::internal_error())?;
            return Ok(());
        }

        self.sess_state.remove(&session_id);
        data.success(resp)
    }

    pub async fn handle_session_close(
        &mut self,
        data: Box<dyn CloseSession>,
        resp: Responder,
    ) -> Result<(), Error> {
        let session_id = data.session_id();
        if !self.sess_state.is_active(&session_id) {
            return data.success(resp);
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
            self.cancel_prompts();
            data.success(resp)
        } else {
            resp.respond_with_error(Error::internal_error())
        }
    }

    pub async fn handle_prompt(
        &mut self,
        data: Box<dyn Prompt>,
        resp: Responder,
    ) -> Result<(), Error> {
        if !self.sess_state.is_active(&data.session_id()) {
            return data.session_not_found(resp);
        }

        let prompt = match data.build_prompt() {
            Ok(p) => p,
            Err(err) => return resp.respond_with_error(err),
        };

        if self.actor.run_prompt(prompt).await.is_err() {
            return resp.respond_with_error(Error::internal_error());
        }

        match self.replies.recv().await {
            Some(CommandReply::PromptQueued { msg_id, .. }) => {
                self.prompts.insert(msg_id, resp);
                Ok(())
            }
            _ => resp.respond_with_error(Error::internal_error()),
        }
    }

    pub async fn handle_notification(&self, note: ACPNotification, _cx: ConnectionTo<Client>) {
        match note {
            ACPNotification::CancelSession => {
                self.cancel_prompts();
                let _ = self.actor.cancel().await;
            }
            ACPNotification::Other => { /* Ignore */ }
        }
    }
}
