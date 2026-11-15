/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    actor::{CommandReply, HarnessActorClient, HarnessCommand, HarnessEvent},
    config::Config,
    harness::{Harness, HarnessMsg},
    session::Session,
    tool_engine::ToolStatus,
};
use agent_client_protocol::{
    Agent, Client, ConnectionTo, Dispatch, Error, HandleDispatchFrom, Handled, JsonRpcResponse,
    Responder, Stdio,
    schema::ProtocolVersion,
    v2::schema::{Error as SchemaError, *},
};
use serde_json::Value;
use std::{
    fmt::Debug,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::{Sender, UnboundedReceiver};

fn reply<T: JsonRpcResponse>(r: Responder<Value>, resp: T) -> Result<(), Error> {
    r.cast::<T>().respond(resp)
}

type SharedSessionId = Arc<Mutex<Option<String>>>;

struct ACPSetup {
    engine: ACPEngine,
    events: UnboundedReceiver<HarnessEvent>,
}

struct ACPEventHandler {
    events: UnboundedReceiver<HarnessEvent>,
    tx: Sender<HarnessCommand>,
    session_id: SharedSessionId,
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
                            if let Some(notification) = self.handle_message(msg) {
                                let _ = cx.send_notification(notification);
                            }
                        }
                        HarnessEvent::AskUser { options, answer_tx, .. } => {
                            // TODO implement real user input
                            let _ = answer_tx.send(options.into_iter().next().unwrap_or_default());
                        }
                        HarnessEvent::TurnFinished{status, ..} => {
                            self.handle_state_update(status, &cx);
                        }
                        HarnessEvent::ShutdownComplete(_) => break
                    }
                }
            }
        }
    }

    fn handle_state_update(&self, status: Result<(), String>, cx: &ConnectionTo<Client>) {
        if let Some(session_id) = self.session_id.lock().unwrap().clone() {
            let idle_state = match status {
                Ok(_) => IdleStateUpdate::new().stop_reason(StopReason::EndTurn),
                Err(msg) => {
                    let error = SchemaError::internal_error().data(msg);
                    let reason = StopReason::Error(ErrorStopReason::new().error(error));
                    IdleStateUpdate::new().stop_reason(reason)
                }
            };
            let update = SessionUpdate::StateUpdate(StateUpdate::Idle(idle_state));
            let msg = UpdateSessionNotification::new(session_id, update);
            let _ = cx.send_notification(msg);
        }
    }

    fn handle_message(&self, msg: HarnessMsg) -> Option<UpdateSessionNotification> {
        let session_id = self.session_id.lock().unwrap().clone()?;
        msg.to_update(&session_id)
    }
}

impl HarnessMsg {
    fn to_update(&self, session_id: &str) -> Option<UpdateSessionNotification> {
        let text = |value: String| ContentBlock::Text(TextContent::new(value));
        let update = match self {
            HarnessMsg::TurnStarted(_) => {
                SessionUpdate::StateUpdate(StateUpdate::Running(RunningStateUpdate::new()))
            }
            HarnessMsg::TurnDuration(_) => return None,
            HarnessMsg::UserPrompt { msg_id, content } => SessionUpdate::UserMessage(
                UserMessage::new(msg_id.to_string()).content(vec![text(content.clone())]),
            ),
            HarnessMsg::AgentMessage { msg_id, content }
            | HarnessMsg::PlanMessage { msg_id, content } => SessionUpdate::AgentMessage(
                AgentMessage::new(msg_id.to_string()).content(vec![text(content.clone())]),
            ),
            HarnessMsg::AgentChunk { msg_id, content } => SessionUpdate::AgentMessageChunk(
                ContentChunk::new(text(content.clone()), msg_id.to_string()),
            ),
            HarnessMsg::Thinking { msg_id, content } => SessionUpdate::AgentThought(
                AgentThought::new(msg_id.to_string()).content(vec![text(content.clone())]),
            ),
            HarnessMsg::ThinkingChunk { msg_id, content } => SessionUpdate::AgentThoughtChunk(
                ContentChunk::new(text(content.clone()), msg_id.to_string()),
            ),
            HarnessMsg::ToolCall {
                call_id,
                name,
                arguments,
                ..
            } => SessionUpdate::ToolCallUpdate(
                ToolCallUpdate::new(call_id.clone())
                    .name(name.clone())
                    .title(name.clone())
                    .raw_input(
                        serde_json::from_str(arguments).unwrap_or(Value::String(arguments.clone())),
                    )
                    .status(ToolCallStatus::InProgress),
            ),
            HarnessMsg::ToolResult {
                call_id,
                name,
                result,
                status,
            } => {
                let status = match status {
                    ToolStatus::Valid => ToolCallStatus::Completed,
                    ToolStatus::Error | ToolStatus::Failure => ToolCallStatus::Failed,
                };
                SessionUpdate::ToolCallUpdate(
                    ToolCallUpdate::new(call_id.clone())
                        .name(name.clone())
                        .status(status)
                        .raw_output(Value::String(result.clone())),
                )
            }
            HarnessMsg::DiffView(_)
            | HarnessMsg::TokenUsage { .. }
            | HarnessMsg::AskUser { .. }
            | HarnessMsg::SystemMessage(_) => return None,
            HarnessMsg::CompactStart { msg_id, .. } => SessionUpdate::CompactionUpdate(
                CompactionUpdate::new(msg_id.to_string(), CompactionStatus::InProgress),
            ),
            HarnessMsg::CompactSummary { msg_id, .. } => SessionUpdate::CompactionUpdate(
                CompactionUpdate::new(msg_id.to_string(), CompactionStatus::Completed),
            ),
            HarnessMsg::QueueChanged(size) => SessionUpdate::Notice(Notice::new(
                NoticeSeverity::Info,
                format!("Queued prompts: {size}"),
            )),
        };
        Some(UpdateSessionNotification::new(session_id, update))
    }
}

struct ACPEngine {
    actor: HarnessActorClient,
    replies: UnboundedReceiver<CommandReply>,
    session_id: SharedSessionId,
}

impl ACPEngine {
    pub async fn setup() -> Result<ACPSetup, Box<dyn core::error::Error>> {
        let mut config = Config::new();
        config.load().await?;

        let harness = Harness::new(&config, None)?;
        let (actor, actor_rx) = HarnessActorClient::new(harness)?;
        let session_id = Arc::new(Mutex::new(None));

        Ok(ACPSetup {
            engine: Self {
                actor,
                replies: actor_rx.replies,
                session_id,
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
            ClientRequest::InitializeRequest(_) => {
                reply(resp, Self::build_init())?;
            }
            ClientRequest::ListSessionsRequest(data) => {
                self.handle_session_list(data, resp).await?
            }
            ClientRequest::NewSessionRequest(data) => self.handle_session_new(data, resp).await?,
            ClientRequest::ResumeSessionRequest(data) => {
                self.handle_session_resume(data, resp, cx).await?
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
                    *self.session_id.lock().unwrap() = Some(session_id.clone());
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
            Some(CommandReply::SessionLoaded(Ok(_))) => {
                *self.session_id.lock().unwrap() = Some(data.session_id.to_string());
            }
            Some(CommandReply::SessionLoaded(Err(_))) | Some(_) | None => {
                resp.respond_with_error(Error::internal_error())?;
                return Ok(());
            }
        }

        if matches!(data.replay_from, Some(ReplayFrom::Start(_))) {
            if self.actor.replay_history().await.is_err() {
                resp.respond_with_error(Error::internal_error())?;
                return Ok(());
            }

            match self.replies.recv().await {
                Some(CommandReply::HistoryReplayed(messages)) => {
                    let session_id = data.session_id.to_string();
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
            *self.session_id.lock().unwrap() = None;
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
    let session_id = engine.session_id.clone();

    Agent
        .v2()
        .name("gaius")
        .with_handler(engine)
        .connect_with(Stdio::default(), async move |cx| {
            ACPEventHandler {
                events,
                tx: actor_tx,
                session_id,
            }
            .run(cx.into())
            .await;
            Ok(())
        })
        .await?;
    Ok(())
}
