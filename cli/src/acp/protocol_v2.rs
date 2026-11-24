/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{protocol::ACPProtocol, reply, state::PendingPrompts},
    actor::FinishedReason,
    commands::Command as GaiusCommand,
    harness::HarnessMsg,
    tool_engine::ToolStatus,
    tools::ToolName,
};
use agent_client_protocol::{
    Client, ConnectionTo, Error, Responder,
    v2::schema::{Error as SchemaError, *},
};
use serde_json::{Value, from_str};
use tokio::sync::oneshot;
use uuid::Uuid;

pub struct ProtocolV2;

impl ProtocolV2 {
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
        let tool_call = ToolCallUpdate::new(ToolCallId::new(call_id))
            .name(name)
            .title(title)
            .kind(kind)
            .status(ToolCallStatus::InProgress)
            .raw_input(json_args);

        SessionUpdate::ToolCallUpdate(tool_call)
    }

    fn to_tool_result(call_id: &str, result: &str, status: ToolStatus) -> SessionUpdate {
        let status = match status {
            ToolStatus::Valid => ToolCallStatus::Completed,
            ToolStatus::Error | ToolStatus::Failure => ToolCallStatus::Failed,
        };
        let block = ContentBlock::Text(TextContent::new(result));
        let content = vec![ToolCallContent::Content(Box::new(Content::new(block)))];
        let tool_call = ToolCallUpdate::new(ToolCallId::new(call_id))
            .status(status)
            .content(content);

        SessionUpdate::ToolCallUpdate(tool_call)
    }

    pub fn to_update(msg: &HarnessMsg, session_id: &str) -> Option<UpdateSessionNotification> {
        let text = |value: String| ContentBlock::Text(TextContent::new(value));
        let update = match msg {
            HarnessMsg::TurnStarted(_) => {
                SessionUpdate::StateUpdate(StateUpdate::Running(RunningStateUpdate::new()))
            }
            HarnessMsg::TurnDuration(_) => return None,
            HarnessMsg::UserPrompt { .. } => {
                // Client already has the prompt because it sent it.
                // Or history replay is done as CommandReply::HistoryReplayed
                return None;
            }
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
            } => Self::to_tool_call(call_id, name, arguments),
            HarnessMsg::ToolResult {
                call_id,
                result,
                status,
                ..
            } => Self::to_tool_result(call_id, result, *status),
            HarnessMsg::DiffView(_)
            | HarnessMsg::TokenUsage { .. }
            | HarnessMsg::AskUser { .. }
            | HarnessMsg::SystemMessage(_) => return None,
            HarnessMsg::CompactStart { msg_id, .. } => SessionUpdate::CompactionUpdate(
                CompactionUpdate::new(msg_id.to_string()).status(CompactionStatus::InProgress),
            ),
            HarnessMsg::CompactSummary { msg_id, content } => {
                let block = ContentBlock::Text(TextContent::new(content));
                SessionUpdate::CompactionUpdate(
                    CompactionUpdate::new(msg_id.to_string())
                        .status(CompactionStatus::Completed)
                        .summary(vec![block]),
                )
            }
            HarnessMsg::QueueChanged(size) => SessionUpdate::Notice(Notice::new(
                NoticeSeverity::Info,
                format!("Queued prompts: {size}"),
            )),
        };
        Some(UpdateSessionNotification::new(session_id, update))
    }
}

impl ACPProtocol for ProtocolV2 {
    fn send_error_notice(
        &self,
        title: String,
        msg: String,
        sess_id: String,
        cx: &ConnectionTo<Client>,
    ) {
        let note = Notice::new(NoticeSeverity::Error, title).description(msg);
        let _ = cx.send_notification(UpdateSessionNotification::new(
            sess_id,
            SessionUpdate::Notice(note),
        ));
    }

    fn send_commands(&self, session_id: &str, cx: &ConnectionTo<Client>) -> Result<(), Error> {
        let commands = vec![AvailableCommand::Command(Command::new(
            GaiusCommand::Info.name(),
            GaiusCommand::Info.description(),
        ))];
        let update = SessionUpdate::AvailableCommandsUpdate(AvailableCommandsUpdate::new(commands));
        cx.send_notification(UpdateSessionNotification::new(session_id, update))
    }

    fn send_agent_msg(&self, session_id: String, content: String, cx: &ConnectionTo<Client>) {
        let msg_id = Uuid::now_v7().to_string();
        let block = ContentBlock::Text(TextContent::new(content));
        let message = AgentMessage::new(msg_id).content(vec![block]);
        let update = SessionUpdate::AgentMessage(message);
        let _ = cx.send_notification(UpdateSessionNotification::new(session_id, update));
    }

    fn send_command_finished(
        &self,
        session_id: String,
        resp: Responder,
        cx: &ConnectionTo<Client>,
    ) -> Result<(), Error> {
        let msg_id = Uuid::now_v7().to_string();
        reply(resp, PromptResponse::new(msg_id))?;

        let idle = IdleStateUpdate::new().stop_reason(StopReason::EndTurn);
        let update = SessionUpdate::StateUpdate(StateUpdate::Idle(idle));
        cx.send_notification(UpdateSessionNotification::new(session_id, update))
    }

    fn is_ack_prompt(&self) -> bool {
        true
    }

    fn ack_prompt(&self, msg_id: u32, resp: Box<Responder>) {
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

    fn on_cancel(&self, responders: Vec<Box<Responder>>) {
        for responder in responders {
            let _ = responder.respond_with_error(Error::request_cancelled());
        }
    }

    fn on_turn_finished(
        &self,
        session_id: String,
        status: FinishedReason,
        _pending: &PendingPrompts,
        cx: &ConnectionTo<Client>,
    ) {
        let idle_state = match status {
            FinishedReason::Done => IdleStateUpdate::new().stop_reason(StopReason::EndTurn),
            FinishedReason::Error(msg) => {
                self.send_error_notice(
                    "Request Failed".into(),
                    msg.clone(),
                    session_id.clone(),
                    cx,
                );
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
