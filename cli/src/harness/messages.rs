/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use super::{Harness, HarnessEvent};
use crate::tool_engine::ToolResult;
use genai::chat::{ChatMessage, ContentPart, CustomPart, MessageContent, ToolCall, Usage};
use serde_json::json;
use std::error::Error;
use uuid::Uuid;

impl Harness {
    pub fn send_system_message<F>(&self, message: String, on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        on_event(HarnessEvent::SystemMessage(message));
    }

    pub fn send_user_message<F>(&mut self, message: String, on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        self.history
            .messages
            .push(ChatMessage::user(message.clone()));
        on_event(HarnessEvent::UserPrompt(message));
    }

    pub fn send_plan_message<F>(&mut self, message: String, on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let marker = ContentPart::Custom(CustomPart {
            model_iden: None,
            data: json!("plan"),
        });
        let content = MessageContent::from_parts(vec![marker, ContentPart::Text(message.clone())]);

        self.history.messages.push(ChatMessage::user(content));
        on_event(HarnessEvent::PlanMessage(message));
    }

    pub fn send_skill_message<F>(
        &mut self,
        name: String,
        mut on_event: F,
    ) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let skill_command = format!("/skill {}", name);
        self.send_user_message(skill_command, &mut on_event);

        let tc = ToolCall {
            call_id: Uuid::new_v4().to_string(),
            fn_name: "skill".to_string(),
            fn_arguments: json!({ "name": name }),
            thought_signatures: None,
        };
        let assistant_content = MessageContent::from_tool_calls(vec![tc.clone()]);
        self.history
            .messages
            .push(ChatMessage::assistant(assistant_content.clone()));

        if let Some(text) = assistant_content.joined_texts() {
            on_event(HarnessEvent::AgentMessage(text));
        }

        self.send_tool_call_event(&tc, &mut on_event);

        let result = self.tool_engine.load_skill(&name);
        match result {
            ToolResult::Text(text) => self.send_tool_result_event(&tc, text, false, &mut on_event),
            ToolResult::Error(err) => self.send_tool_result_event(&tc, err, true, &mut on_event),
            _ => {
                return Err("Tool engine failed to run skill".into());
            }
        }

        Ok(())
    }

    pub fn drop_queued_prompts<F>(&self, on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let dropped = self.prompt_queue.clear();
        if dropped > 0 {
            on_event(HarnessEvent::QueueChanged(0));
            let plural = if dropped == 1 { "" } else { "s" };
            on_event(HarnessEvent::SystemMessage(format!(
                "Dropped {dropped} queued prompt{plural}"
            )));
        }
    }

    pub fn record_usage<F>(
        &mut self,
        prompt_idx: usize,
        response_idx: usize,
        usage_opt: Option<Usage>,
        on_event: &mut F,
    ) where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let Some(usage) = usage_opt else {
            return;
        };

        let pricing = self.client.model().pricing.as_ref();
        self.token_usage
            .record_with_event(prompt_idx, response_idx, &usage, pricing, on_event);
    }

    /// Record the usage of a side request — one whose messages are not part of
    /// the chat history, like the compaction summary call — so its tokens and
    /// cost are counted in the session usage.
    pub fn record_side_usage<F>(&mut self, usage: &Usage, on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let pricing = self.client.model().pricing.as_ref();
        self.token_usage.record_side_usage(usage, pricing);
        on_event(HarnessEvent::TokenUsage {
            prompt: usage.prompt_tokens,
            response: usage.completion_tokens,
            // A side request (such as compaction) does not change the active
            // conversation context. `apply_compaction` emits the new total
            // after it replaces the history.
            total: None,
            cost: self.token_usage.usage().total_cost(),
        });
        self.update_session_info();
    }
}
