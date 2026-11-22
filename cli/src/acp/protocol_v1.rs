/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{protocol::ACPProtocol, reply, state::PendingPrompts},
    actor::FinishedReason,
    harness::HarnessMsg,
    tool_engine::ToolStatus,
    tools::ToolName,
};
use agent_client_protocol::{Client, ConnectionTo, Responder, v1::schema::*};
use serde_json::{Value, from_str};
use tokio::sync::oneshot;

pub struct ProtocolV1;

impl ProtocolV1 {
    fn to_tool_call(call_id: &str, name: &str, args: &str) -> SessionUpdate {
        let tool_name = ToolName::from_name(name);
        let kind = tool_name.map_or(ToolKind::Other, |tool| match tool {
            ToolName::Read => ToolKind::Read,
            ToolName::Write | ToolName::Edit => ToolKind::Edit,
            ToolName::Bash => ToolKind::Execute,
            ToolName::Glob | ToolName::Grep => ToolKind::Search,
            ToolName::WebFetch | ToolName::WebSearch => ToolKind::Fetch,
            _ => ToolKind::Other,
        });
        let json_args = from_str::<Value>(args).unwrap_or_default();
        let title = format!("{} {}", name, ToolName::desc(tool_name, &json_args));
        let tool_call = ToolCall::new(ToolCallId::new(call_id), title)
            .name(name)
            .kind(kind)
            .status(ToolCallStatus::InProgress)
            .raw_input(json_args);

        SessionUpdate::ToolCall(tool_call)
    }

    fn to_tool_result(call_id: &str, result: &str, status: ToolStatus) -> SessionUpdate {
        let status = match status {
            ToolStatus::Valid => ToolCallStatus::Completed,
            ToolStatus::Error | ToolStatus::Failure => ToolCallStatus::Failed,
        };
        let block = ContentBlock::Text(TextContent::new(result));
        let content = vec![ToolCallContent::Content(Content::new(block))];
        let tool_call = ToolCallUpdate::new(
            ToolCallId::new(call_id),
            ToolCallUpdateFields::new().status(status).content(content),
        );

        SessionUpdate::ToolCallUpdate(tool_call)
    }

    pub fn to_update(msg: &HarnessMsg, session_id: &str) -> Option<SessionNotification> {
        let text = |s: String| ContentBlock::Text(TextContent::new(s));
        let update = match msg {
            HarnessMsg::UserPrompt { .. } => {
                // Client already has the prompt because it sent it.
                // Or history replay is done as CommandReply::HistoryReplayed
                return None;
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
            } => Self::to_tool_call(call_id, name, arguments),
            HarnessMsg::ToolResult {
                call_id,
                result,
                status,
                ..
            } => Self::to_tool_result(call_id, result, *status),
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
