/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    actor::{CommandReply, HarnessActorClient, HarnessCommand, HarnessEvent},
    config::Config,
    harness::{Harness, HarnessMsg, HarnessSnapshot},
    session::Session,
    tool_engine::ToolStatus,
};
use agent_client_protocol::{
    Agent, Client, ConnectionTo, Dispatch, Error, HandleDispatchFrom, Handled, JsonRpcResponse,
    Responder, Stdio, schema::ProtocolVersion, v2::schema::*,
};
use serde_json::Value;
use std::fmt::Debug;
use tokio::sync::mpsc::{Sender, UnboundedReceiver};

fn reply<T: JsonRpcResponse>(r: Responder<Value>, resp: T) -> Result<(), Error> {
    r.cast::<T>().respond(resp)
}

struct ACPSetup {
    engine: ACPEngine,
    events: UnboundedReceiver<HarnessEvent>,
}

struct ACPEventHandler {
    events: UnboundedReceiver<HarnessEvent>,
    tx: Sender<HarnessCommand>,
}

impl ACPEventHandler {
    async fn run(mut self, cx: ConnectionTo<Client>) {
        loop {
            tokio::select! {
                _ = cx.incoming_closed() => {
                    HarnessActorClient::shutdown(self.tx, &mut self.events).await;
                    break;
                },
                event = self.events.recv() => {
                    let Some(event) = event else { break; };
                    match event {
                        HarnessEvent::Message(msg) => {
                            if let Some(notification) = Self::handle_message(msg) {
                                let _ = cx.send_notification(notification);
                            }
                        }
                        HarnessEvent::AskUser { .. } => {}
                        HarnessEvent::TurnFinished(snapshot) => {
                            Self::handle_turn_finished(snapshot, &cx);
                        }
                        HarnessEvent::RequestFailed(_, snapshot) => {
                            Self::handle_request_failed(snapshot, &cx);
                        }
                        HarnessEvent::ShutdownComplete(_) => break
                    }
                }
            }
        }
    }

    fn handle_turn_finished(snapshot: HarnessSnapshot, cx: &ConnectionTo<Client>) {
        if let Some(session_id) = snapshot.session_id {
            let update =
                SessionUpdate::StateUpdate(StateUpdate::Running(RunningStateUpdate::new()));
            let msg = UpdateSessionNotification::new(session_id, update);
            let _ = cx.send_notification(msg);
        }
    }

    fn handle_request_failed(snapshot: HarnessSnapshot, cx: &ConnectionTo<Client>) {
        if let Some(session_id) = snapshot.session_id {
            let update = SessionUpdate::StateUpdate(StateUpdate::Idle(IdleStateUpdate::new()));
            let msg = UpdateSessionNotification::new(session_id, update);
            let _ = cx.send_notification(msg);
        }
    }

    fn handle_message(msg: HarnessMsg) -> Option<UpdateSessionNotification> {
        let text = |value: String| ContentBlock::Text(TextContent::new(value));
        let update = match msg {
            HarnessMsg::TurnStarted(_) => {
                SessionUpdate::StateUpdate(StateUpdate::Running(RunningStateUpdate::new()))
            }
            HarnessMsg::TurnDuration(_) => return None,
            HarnessMsg::UserPrompt(value) => SessionUpdate::UserMessage(
                UserMessage::new("dummy-message-id").content(vec![text(value)]),
            ),
            HarnessMsg::AgentMessage(value) | HarnessMsg::PlanMessage(value) => {
                SessionUpdate::AgentMessage(
                    AgentMessage::new("dummy-message-id").content(vec![text(value)]),
                )
            }
            HarnessMsg::Thinking(value) => SessionUpdate::AgentThought(
                AgentThought::new("dummy-message-id").content(vec![text(value)]),
            ),
            HarnessMsg::ToolCall {
                name, arguments, ..
            } => SessionUpdate::ToolCallUpdate(
                ToolCallUpdate::new("dummy-tool-call-id")
                    .name(name.clone())
                    .title(name)
                    .raw_input(serde_json::from_str(&arguments).unwrap_or(Value::String(arguments)))
                    .status(ToolCallStatus::InProgress),
            ),
            HarnessMsg::ToolResult {
                name,
                result,
                status,
            } => {
                let status = match status {
                    ToolStatus::Valid => ToolCallStatus::Completed,
                    ToolStatus::Error | ToolStatus::Failure => ToolCallStatus::Failed,
                };
                SessionUpdate::ToolCallUpdate(
                    ToolCallUpdate::new("dummy-tool-call-id")
                        .name(name)
                        .status(status)
                        .raw_output(Value::String(result)),
                )
            }
            HarnessMsg::DiffView(_) => return None,
            HarnessMsg::TokenUsage { .. } => return None,
            HarnessMsg::AskUser { .. } => return None,
            HarnessMsg::CompactStart { .. } => SessionUpdate::CompactionUpdate(
                CompactionUpdate::new("compaction_id", CompactionStatus::InProgress),
            ),
            HarnessMsg::CompactSummary(_) => SessionUpdate::CompactionUpdate(
                CompactionUpdate::new("compaction_id", CompactionStatus::Completed),
            ),
            HarnessMsg::QueueChanged(size) => SessionUpdate::Notice(Notice::new(
                NoticeSeverity::Info,
                format!("Queued prompts: {size}"),
            )),
            HarnessMsg::SystemMessage(_) => return None,
        };
        Some(UpdateSessionNotification::new("dummy-session-id", update))
    }
}

struct ACPEngine {
    actor: HarnessActorClient,
    replies: UnboundedReceiver<CommandReply>,
}

impl ACPEngine {
    pub async fn setup() -> Result<ACPSetup, Box<dyn core::error::Error>> {
        let mut config = Config::new();
        config.load().await?;

        let harness = Harness::new(&config, None)?;
        let (actor, actor_rx) = HarnessActorClient::new(harness)?;

        Ok(ACPSetup {
            engine: Self {
                actor,
                replies: actor_rx.replies,
            },
            events: actor_rx.events,
        })
    }

    pub async fn handle_request(
        &mut self,
        req: ClientRequest,
        resp: Responder,
        _cx: ConnectionTo<Client>,
    ) -> Result<(), Error> {
        match req {
            ClientRequest::InitializeRequest(_) => {
                reply(resp, Self::build_init())?;
            }
            ClientRequest::ListSessionsRequest(data) => {
                self.handle_session_list(data, resp).await?
            }
            ClientRequest::NewSessionRequest(data) => self.handle_session_new(data, resp).await?,
            ClientRequest::ResumeSessionRequest(data) => {
                self.handle_session_resume(data, resp).await?
            }
            ClientRequest::CloseSessionRequest(_) => self.handle_session_close(resp).await?,
            ClientRequest::DeleteSessionRequest(data) => {
                if Session::delete(&data.session_id.to_string()).is_err() {
                    resp.respond_with_error(Error::internal_error())?;
                } else {
                    reply(resp, DeleteSessionResponse::new())?;
                }
            }
            ClientRequest::SetSessionConfigOptionRequest(_) => {
                reply(resp, SetSessionConfigOptionResponse::new(vec![]))?;
            }
            ClientRequest::PromptRequest(data) => self.handle_prompt(data, resp).await?,
            _ => resp.respond_with_error(Error::internal_error())?,
        }
        Ok(())
    }

    pub async fn handle_session_list(
        &mut self,
        data: Box<ListSessionsRequest>,
        resp: Responder,
    ) -> Result<(), Error> {
        let cwd = data.cwd.map(|cwd| cwd.0);
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
        let cwd = Some(data.cwd.0);
        if self.actor.new_session(cwd).await.is_err() {
            resp.respond_with_error(Error::internal_error())?;
            return Ok(());
        }

        match self.replies.recv().await {
            Some(CommandReply::SessionCreated(Ok(snapshot))) => {
                if let Some(session_id) = snapshot.session_id {
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
    ) -> Result<(), Error> {
        if self
            .actor
            .load_session(data.session_id.to_string())
            .await
            .is_err()
        {
            resp.respond_with_error(Error::internal_error())?;
            return Ok(());
        }

        match self.replies.recv().await {
            Some(CommandReply::SessionLoaded(result)) => result.is_ok(),
            Some(_) | None => {
                resp.respond_with_error(Error::internal_error())?;
                return Ok(());
            }
        };

        if matches!(data.replay_from, Some(ReplayFrom::Start(_))) {
            if self.actor.replay_history().await.is_err() {
                resp.respond_with_error(Error::internal_error())?;
                return Ok(());
            }

            match self.replies.recv().await {
                Some(CommandReply::HistoryReplayed(_)) => {}
                Some(CommandReply::Busy(_)) | Some(_) | None => {
                    resp.respond_with_error(Error::internal_error())?;
                    return Ok(());
                }
            }
        }

        // must send UpdateSessionNotification with running note.
        reply(resp, ResumeSessionResponse::new())
    }

    pub async fn handle_session_close(&mut self, resp: Responder) -> Result<(), Error> {
        if self.actor.close_session().await.is_err() {
            resp.respond_with_error(Error::internal_error())?;
            return Ok(());
        }
        let closed = match self.replies.recv().await {
            Some(CommandReply::SessionClosed(result)) => result.is_ok(),
            Some(_) | None => false,
        };
        if closed {
            reply(resp, CloseSessionResponse::new())
        } else {
            resp.respond_with_error(Error::internal_error())
        }
    }

    pub async fn handle_prompt(
        &self,
        data: Box<PromptRequest>,
        resp: Responder,
    ) -> Result<(), Error> {
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
                _ => {}
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
            resp.respond_with_error(Error::internal_error())
        } else {
            reply(resp, PromptResponse::new("message_id"))
        }
    }

    pub async fn handle_notification(&self, note: ClientNotification, _cx: ConnectionTo<Client>) {
        match note {
            ClientNotification::CancelSessionNotification(_) => {
                let _ = self.actor.cancel().await;
            }
            _ => { /* Ignore */ }
        }
    }

    fn build_init() -> InitializeResponse {
        let ver = Implementation::new("gaius", env!("GIT_VERSION"));
        let msg = InitializeResponse::new(ProtocolVersion::V2, ver);
        let sess = SessionCapabilities::new().delete(SessionDeleteCapabilities::new());
        let cap = AgentCapabilities::new().session(sess);
        msg.capabilities(cap)
    }
}

impl HandleDispatchFrom<Client> for ACPEngine {
    async fn handle_dispatch_from(
        &mut self,
        message: Dispatch,
        cx: ConnectionTo<Client>,
    ) -> Result<Handled<Dispatch>, Error> {
        let message = match message.into_request::<ClientRequest>()? {
            Ok((req, responder)) => {
                self.handle_request(req, responder, cx).await?;
                return Ok(Handled::Yes);
            }
            Err(message) => message,
        };

        let message = match message.into_notification::<ClientNotification>()? {
            Ok(notif) => {
                self.handle_notification(notif, cx).await;
                return Ok(Handled::Yes);
            }
            Err(message) => message,
        };

        Ok(Handled::No {
            message,
            retry: false,
        })
    }

    fn describe_chain(&self) -> impl Debug {
        "Gaius"
    }
}

pub async fn run_acp() -> Result<(), Box<dyn core::error::Error>> {
    let ACPSetup { engine, events } = match ACPEngine::setup().await {
        Ok(setup) => setup,
        Err(error) => {
            return Err(error);
        }
    };
    let actor_tx = engine.actor.clone_tx();

    Agent
        .v2()
        .name("gaius")
        .with_handler(engine)
        .connect_with(Stdio::default(), async move |cx| {
            ACPEventHandler {
                events,
                tx: actor_tx,
            }
            .run(cx.into())
            .await;
            Ok(())
        })
        .await?;
    Ok(())
}
