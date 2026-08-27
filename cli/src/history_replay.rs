/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{diff_view::DiffView, harness::HarnessEvent, token_usage::TokenUsageLedger};
use genai::chat::{ChatMessage, ChatRole, ContentPart, CustomPart};

pub trait MessageExt {
    fn is_tool_error(&self) -> bool;

    fn has_plan_marker(&self) -> bool;

    fn has_compact_summary_marker(&self) -> bool;

    fn emit_diff_markers<F>(&self, on_event: &mut F)
    where
        F: FnMut(HarnessEvent);
}

impl MessageExt for ChatMessage {
    fn is_tool_error(&self) -> bool {
        self.content.custom_parts().iter().any(|part| {
            part.data
                .as_object()
                .is_some_and(|obj: &serde_json::Map<String, serde_json::Value>| {
                    obj.get("tool_error") == Some(&serde_json::json!(true))
                })
        })
    }

    fn has_plan_marker(&self) -> bool {
        matches!(
            self.content.parts().first(),
            Some(ContentPart::Custom(CustomPart { data, .. })) if data == &serde_json::json!("plan")
        )
    }

    fn has_compact_summary_marker(&self) -> bool {
        matches!(
            self.content.parts().first(),
            Some(ContentPart::Custom(CustomPart { data, .. }))
                if data == &serde_json::json!("compact_summary")
        )
    }

    fn emit_diff_markers<F>(&self, on_event: &mut F)
    where
        F: FnMut(HarnessEvent),
    {
        for part in self.content.custom_parts() {
            if let Some(diff) = DiffView::from_marker(&part.data) {
                on_event(HarnessEvent::DiffView(diff));
            }
        }
    }
}

/// Replay a slice of chat history as `HarnessEvent` callbacks, pairing
/// assistant tool-calls with their following tool-response messages.
///
/// TUI and CLI callers can use this as the single code path for rendering
/// both live turns and previously-saved history.
pub fn replay_messages<F>(history: &[ChatMessage], token_usage: &TokenUsageLedger, mut on_event: F)
where
    F: FnMut(HarnessEvent),
{
    let mut pending_tool_calls: Vec<(String, String)> = Vec::new();
    let mut messages = history.iter().enumerate().peekable();

    while let Some((index, message)) = messages.next() {
        match message.role {
            ChatRole::User => {
                pending_tool_calls.clear();
                let text = message.content.texts().join("");
                if !text.is_empty() {
                    if message.has_plan_marker() {
                        on_event(HarnessEvent::PlanMessage(text));
                    } else if message.has_compact_summary_marker() {
                        on_event(HarnessEvent::CompactStart { start_time: 0 });
                        on_event(HarnessEvent::CompactSummary(text));
                    } else {
                        on_event(HarnessEvent::UserPrompt(text));
                    }
                }
                token_usage.emit_usage(index, &mut on_event);
            }
            ChatRole::Assistant => {
                let text = message.content.texts().join("");

                // Emit any stored thinking/reasoning content first
                for part in message.content.parts() {
                    match part {
                        ContentPart::ThoughtSignature(text)
                        | ContentPart::ReasoningContent(text)
                            if !text.is_empty() =>
                        {
                            on_event(HarnessEvent::Thinking(text.clone()));
                        }
                        _ => {}
                    }
                }

                // Collect pending tool calls from this assistant turn
                for tc in message.content.tool_calls() {
                    pending_tool_calls.push((tc.fn_name.clone(), tc.fn_arguments.to_string()));
                }

                if !text.is_empty() {
                    on_event(HarnessEvent::AgentMessage(text));
                }
                token_usage.emit_usage(index, &mut on_event);

                // Match consecutive Tool-role response messages to the pending
                // tool calls in order.
                loop {
                    let is_tool = match messages.peek() {
                        Some((_, m)) => m.role == ChatRole::Tool,
                        None => false,
                    };
                    if !is_tool {
                        break;
                    }
                    let (next_index, next_msg) = messages.next().unwrap();
                    let responses: Vec<&genai::chat::ToolResponse> =
                        next_msg.content.tool_responses();
                    let tool_error = next_msg.is_tool_error();
                    for resp in responses {
                        if let Some((name, args)) = pending_tool_calls.first() {
                            on_event(HarnessEvent::ToolCall {
                                name: (*name).clone(),
                                arguments: (*args).clone(),
                                start_time: 0,
                            });
                            on_event(HarnessEvent::ToolResult {
                                name: (*name).clone(),
                                result: resp.content.clone(),
                                error: tool_error,
                            });
                            pending_tool_calls.remove(0);
                        }
                    }
                    next_msg.emit_diff_markers(&mut on_event);
                    token_usage.emit_usage(next_index, &mut on_event);
                }

                // Any remaining unmatched calls — emit with empty result so
                // the UI always renders something.
                for (name, args) in pending_tool_calls.drain(..) {
                    on_event(HarnessEvent::ToolCall {
                        name: name.clone(),
                        arguments: args,
                        start_time: 0,
                    });
                    on_event(HarnessEvent::ToolResult {
                        name,
                        result: String::new(),
                        error: false,
                    });
                }
            }
            ChatRole::Tool => {
                // Unmatched tool response — display inline as agent text.
                let text = message.content.texts().join("");
                if !text.is_empty() {
                    on_event(HarnessEvent::AgentMessage(text));
                }
                for tr in message.content.tool_responses() {
                    on_event(HarnessEvent::AgentMessage(format!(
                        "[tool {}]: {}",
                        tr.call_id, tr.content
                    )));
                }
                message.emit_diff_markers(&mut on_event);
                token_usage.emit_usage(index, &mut on_event);
            }
            ChatRole::System => {
                pending_tool_calls.clear();
                token_usage.emit_usage(index, &mut on_event);
            }
        }
    }
}
