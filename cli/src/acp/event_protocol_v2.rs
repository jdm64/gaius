/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{event_handler::ACPEventProtocol, util::reply},
    actor::FinishedReason,
    harness::HarnessMsg,
    tool_engine::ToolStatus,
};
use agent_client_protocol::{
    Client, ConnectionTo, Responder,
    v2::schema::{Error as SchemaError, *},
};
use serde_json::Value;
use tokio::sync::oneshot;

pub struct EventProtocolV2;

impl EventProtocolV2 {
    pub fn to_update(msg: &HarnessMsg, session_id: &str) -> Option<UpdateSessionNotification> {
        let text = |value: String| ContentBlock::Text(TextContent::new(value));
        let update = match msg {
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
                    .raw_input(Value::String(arguments.clone()))
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

impl ACPEventProtocol for EventProtocolV2 {
    fn on_prompt(&self, msg_id: u32, resp: Box<Responder>) {
        let msg = PromptResponse::new(msg_id.to_string());
        let _ = reply(*resp, msg);
    }

    fn on_message(&self, session_id: String, msg: &HarnessMsg, cx: &ConnectionTo<Client>) {
        if let Some(notification) = Self::to_update(msg, &session_id) {
            let _ = cx.send_notification(notification);
        }
    }

    fn on_ask_user(
        &self,
        session_id: String,
        call_id: String,
        title: String,
        options: Vec<String>,
        answer_tx: oneshot::Sender<String>,
        cx: &ConnectionTo<Client>,
    ) {
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

    fn on_turn_finished(
        &self,
        session_id: String,
        status: FinishedReason,
        cx: &ConnectionTo<Client>,
    ) {
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
}
