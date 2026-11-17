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
use chrono::Utc;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::{
    Notify,
    mpsc::{Sender, UnboundedReceiver},
    oneshot,
};

fn reply<T: JsonRpcResponse>(r: Responder<Value>, resp: T) -> Result<(), Error> {
    r.cast::<T>().respond(resp)
}

fn reply_no_session(resp: Responder, id: SessionId) -> Result<(), Error> {
    resp.respond_with_error(SchemaError::session_not_found(id).into())
}

fn abs_path(path: Option<AbsolutePath>) -> Result<Option<PathBuf>, ()> {
    match path {
        Some(p) if p.0.is_absolute() => Ok(Some(p.0)),
        None => Ok(None),
        _ => Err(()),
    }
}

type SharedSessionId = Arc<Mutex<Option<String>>>;

#[derive(Default, Clone)]
struct SessState {
    active: SharedSessionId,
    known: HashSet<String>,
}

impl SessState {
    fn get(shared: &SharedSessionId) -> Option<String> {
        shared.lock().unwrap().clone()
    }

    fn set(&mut self, sess_id: &str) {
        *self.active.lock().unwrap() = Some(sess_id.to_string());
        self.known.insert(sess_id.to_string());
    }

    fn is_active(&self, sess_id: &str) -> bool {
        self.active.lock().unwrap().as_deref() == Some(sess_id)
    }

    fn is_unknown(&self, sess_id: &str) -> bool {
        !self.is_active(sess_id) && !self.known.contains(sess_id) && !Session::exists(sess_id)
    }

    fn is_active_id(&self, id: &SessionId) -> bool {
        self.is_active(&id.to_string())
    }

    fn clear(&mut self) {
        *self.active.lock().unwrap() = None;
    }

    fn remove(&mut self, sess_id: &str) {
        self.known.remove(sess_id);
    }
}

enum PendingPrompt {
    Waiting(Responder),
    Cancelled,
}

#[derive(Default, Clone)]
struct PendingPrompts {
    arc: Arc<(Mutex<HashMap<u32, PendingPrompt>>, Notify)>,
}

impl PendingPrompts {
    fn insert(&self, msg_id: u32, responder: Responder) {
        self.arc
            .0
            .lock()
            .unwrap()
            .insert(msg_id, PendingPrompt::Waiting(responder));
        self.arc.1.notify_waiters();
    }

    async fn respond(&self, msg_id: u32) {
        loop {
            let notified = self.arc.1.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let slot = self.arc.0.lock().unwrap().remove(&msg_id);
            match slot {
                Some(PendingPrompt::Waiting(responder)) => {
                    let _ = reply(responder, PromptResponse::new(msg_id.to_string()));
                    return;
                }
                Some(PendingPrompt::Cancelled) => return,
                None => notified.await,
            }
        }
    }

    fn cancel(&self) {
        let responders: Vec<Responder> = {
            let mut prompts = self.arc.0.lock().unwrap();
            prompts
                .values_mut()
                .filter_map(
                    |slot| match std::mem::replace(slot, PendingPrompt::Cancelled) {
                        PendingPrompt::Waiting(responder) => Some(responder),
                        PendingPrompt::Cancelled => None,
                    },
                )
                .collect()
        };
        for responder in responders {
            let _ = responder.respond_with_error(Error::request_cancelled());
        }
        self.arc.1.notify_waiters();
    }
}

struct ACPSetup {
    engine: ACPEngine,
    events: UnboundedReceiver<HarnessEvent>,
}

struct ACPEventHandler {
    events: UnboundedReceiver<HarnessEvent>,
    tx: Sender<HarnessCommand>,
    session_id: SharedSessionId,
    pending_prompts: PendingPrompts,
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
                            if let HarnessMsg::UserPrompt { msg_id, .. } = &msg {
                                self.pending_prompts.respond(*msg_id).await;
                            }
                            if let Some(notification) = self.handle_message(msg) {
                                let _ = cx.send_notification(notification);
                            }
                        }
                        HarnessEvent::AskUser { call_id, title, options, answer_tx } => {
                            self.handle_ask_user(call_id, title, options, answer_tx, &cx);
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

    fn handle_ask_user(
        &self,
        call_id: String,
        title: String,
        options: Vec<String>,
        answer_tx: oneshot::Sender<String>,
        cx: &ConnectionTo<Client>,
    ) {
        let Some(session_id) = SessState::get(&self.session_id) else {
            let _ = answer_tx.send(String::new());
            return;
        };
        let scope = ElicitationSessionScope::new(session_id).tool_call_id(call_id.as_str());
        let values = StringPropertySchema::new().enum_values(options);
        let schema = ElicitationSchema::new()
            .title(title.clone())
            .property("choice", values, true);
        let mode = ElicitationFormMode::new(ElicitationScope::Session(scope), schema);
        let request = CreateElicitationRequest::new(ElicitationMode::Form(mode), title);
        let sent = cx.send_request(request);
        tokio::spawn(async move {
            let answer = match sent.block_task().await {
                Ok(response) => match response.action {
                    ElicitationAction::Accept(accepted) => accepted
                        .content
                        .and_then(|content| content.get("choice").cloned())
                        .and_then(|choice| serde_json::to_value(choice).ok())
                        .and_then(|choice| choice.as_str().map(str::to_owned))
                        .unwrap_or_default(),
                    ElicitationAction::Decline
                    | ElicitationAction::Cancel
                    | ElicitationAction::Other(_) => String::new(),
                    _ => String::new(),
                },
                Err(_) => String::new(),
            };
            let _ = answer_tx.send(answer);
        });
    }

    fn handle_state_update(&self, status: Result<(), String>, cx: &ConnectionTo<Client>) {
        if let Some(session_id) = SessState::get(&self.session_id) {
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
        let session_id = SessState::get(&self.session_id)?;
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
    sess_state: SessState,
    pending_prompts: PendingPrompts,
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
                        &vec![ProtocolVersion::V2],
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
            Some(from) if matches!(from, ReplayFrom::Start(_)) => true,
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
    let session_id = engine.sess_state.active.clone();
    let pending_prompts = engine.pending_prompts.clone();

    Agent
        .v2()
        .name("gaius")
        .with_handler(engine)
        .connect_with(Stdio::default(), async move |cx| {
            ACPEventHandler {
                events,
                tx: actor_tx,
                session_id,
                pending_prompts,
            }
            .run(cx.into())
            .await;
            Ok(())
        })
        .await?;
    Ok(())
}
