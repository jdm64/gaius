/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{protocol::ACPProtocol, reply, state::PendingPrompts},
    actor::FinishedReason,
    harness::HarnessMsg,
    tool_engine::ToolStatus,
};
use agent_client_protocol::{Client, ConnectionTo, Responder, v1::schema::*};
use serde_json::Value;
use tokio::sync::oneshot;

pub struct ProtocolV1;

impl ProtocolV1 {
    pub fn to_update(msg: &HarnessMsg, session_id: &str) -> Option<SessionNotification> {
        let text = |s: String| ContentBlock::Text(TextContent::new(s));
        let update = match msg {
            HarnessMsg::UserPrompt { content, .. } => {
                SessionUpdate::UserMessageChunk(ContentChunk::new(text(content.clone())))
            }
            HarnessMsg::AgentMessage { content, .. } | HarnessMsg::PlanMessage { content, .. } => {
                SessionUpdate::AgentMessageChunk(ContentChunk::new(text(content.clone())))
            }
            HarnessMsg::AgentChunk { content, .. } => {
                SessionUpdate::AgentMessageChunk(ContentChunk::new(text(content.clone())))
            }
            HarnessMsg::Thinking { content, .. } | HarnessMsg::ThinkingChunk { content, .. } => {
                SessionUpdate::AgentThoughtChunk(ContentChunk::new(text(content.clone())))
            }
            HarnessMsg::ToolCall {
                call_id,
                name,
                arguments,
                ..
            } => SessionUpdate::ToolCall(
                ToolCall::new(call_id.clone(), name.clone())
                    .name(name.clone())
                    .raw_input(Value::String(arguments.clone())),
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
                SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    call_id.clone(),
                    ToolCallUpdateFields::new()
                        .name(name.clone())
                        .status(status)
                        .raw_output(Value::String(result.clone())),
                ))
            }
            HarnessMsg::QueueChanged(_)
            | HarnessMsg::TurnStarted(_)
            | HarnessMsg::TurnDuration(_)
            | HarnessMsg::DiffView(_)
            | HarnessMsg::TokenUsage { .. }
            | HarnessMsg::AskUser { .. }
            | HarnessMsg::SystemMessage(_)
            | HarnessMsg::CompactStart { .. }
            | HarnessMsg::CompactSummary { .. } => return None,
        };
        Some(SessionNotification::new(session_id.to_string(), update))
    }
}

impl ACPProtocol for ProtocolV1 {
    fn send_error_notice(
        &self,
        title: String,
        msg: String,
        sess_id: String,
        cx: &ConnectionTo<Client>,
    ) {
        let note = Notice::new(NoticeSeverity::Error, title).description(msg);
        let _ = cx.send_notification(SessionNotification::new(
            sess_id,
            SessionUpdate::Notice(note),
        ));
    }

    fn is_ack_prompt(&self) -> bool {
        false
    }

    fn ack_prompt(&self, _: u32, _: Box<Responder>) {
        // V1 responds only when the turn finishes in on_turn_finished().
    }

    fn on_message(&self, session_id: String, msg: &HarnessMsg, cx: &ConnectionTo<Client>) {
        if let Some(note) = Self::to_update(msg, &session_id) {
            let _ = cx.send_notification(note);
        }
    }

    fn on_ask_user(
        &self,
        _: String,
        _: String,
        _: String,
        _: Vec<String>,
        tx: oneshot::Sender<String>,
        _: &ConnectionTo<Client>,
    ) {
        let _ = tx.send(String::new());
    }

    fn on_cancel(&self, mut responders: Vec<Box<Responder>>) {
        if let Some(responder) = responders.drain(..).next() {
            let _ = reply(*responder, PromptResponse::new(StopReason::Cancelled));
        }
    }

    fn on_turn_finished(
        &self,
        sess_id: String,
        status: FinishedReason,
        pending: &PendingPrompts,
        cx: &ConnectionTo<Client>,
    ) {
        let mut error = None;
        let reason = match status {
            FinishedReason::Done => StopReason::EndTurn,
            FinishedReason::Cancel => StopReason::Cancelled,
            FinishedReason::Error(e) => {
                error = Some(e);
                StopReason::Cancelled
            }
        };

        let msg = PromptResponse::new(reason);
        let msg = if let Some(error_msg) = error {
            self.send_error_notice("Request Failed".into(), error_msg.clone(), sess_id, cx);
            let mut data = Meta::new();
            data.insert("error".to_string(), error_msg.into());
            msg.meta(data)
        } else {
            msg
        };

        pending.ack_first(|responder| {
            let _ = reply(*responder, msg);
        });
    }
}
