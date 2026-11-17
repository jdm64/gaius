/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::state::{PendingPrompts, SessState, SharedSessionId},
    actor::{FinishedReason, HarnessActorClient, HarnessCommand, HarnessEvent},
    harness::HarnessMsg,
    tool_engine::ToolStatus,
};
use agent_client_protocol::{
    Client, ConnectionTo,
    v2::schema::{Error as SchemaError, *},
};
use serde_json::Value;
use tokio::sync::{
    mpsc::{Sender, UnboundedReceiver},
    oneshot,
};

impl HarnessMsg {
    pub fn to_update(&self, session_id: &str) -> Option<UpdateSessionNotification> {
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

pub struct ACPEventHandler {
    events: UnboundedReceiver<HarnessEvent>,
    tx: Sender<HarnessCommand>,
    session_id: SharedSessionId,
    pending_prompts: PendingPrompts,
}

impl ACPEventHandler {
    pub fn new(
        events: UnboundedReceiver<HarnessEvent>,
        tx: Sender<HarnessCommand>,
        session_id: SharedSessionId,
        pending_prompts: PendingPrompts,
    ) -> Self {
        Self {
            events,
            tx,
            session_id,
            pending_prompts,
        }
    }

    pub async fn run(mut self, cx: ConnectionTo<Client>) {
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
                        HarnessEvent::TurnFinished { status, ack, .. } => {
                            self.handle_state_update(status, &cx);
                            if let Some(ack) = ack {
                                let _ = ack.send(());
                            }
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

    fn handle_state_update(&self, status: FinishedReason, cx: &ConnectionTo<Client>) {
        let Some(session_id) = SessState::get(&self.session_id) else {
            return;
        };
        let idle_state = match status {
            FinishedReason::Done => IdleStateUpdate::new().stop_reason(StopReason::EndTurn),
            FinishedReason::Error(msg) => {
                let error = SchemaError::internal_error().data(msg);
                let reason = StopReason::Error(ErrorStopReason::new().error(error));
                IdleStateUpdate::new().stop_reason(reason)
            }
            FinishedReason::Cancel => IdleStateUpdate::new().stop_reason(StopReason::Cancelled),
        };
        let update = SessionUpdate::StateUpdate(StateUpdate::Idle(idle_state));
        let msg = UpdateSessionNotification::new(session_id, update);
        let _ = cx.send_notification(msg);
    }

    fn handle_message(&self, msg: HarnessMsg) -> Option<UpdateSessionNotification> {
        let session_id = SessState::get(&self.session_id)?;
        msg.to_update(&session_id)
    }
}
