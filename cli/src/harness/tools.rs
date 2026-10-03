/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use super::{Harness, HarnessEvent};
use crate::{
    diff_view::DiffView, render::util::RenderUtil, tool_engine::ToolResult, util::time_now,
};
use genai::chat::{ChatMessage, ContentPart, CustomPart, ToolCall, ToolResponse};
use serde_json::json;

impl Harness {
    pub(super) async fn call_tools<F>(&mut self, tool_calls: &[ToolCall], on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        for tc in tool_calls {
            if self.is_cancel() {
                return;
            }

            self.send_tool_call_event(tc, on_event);

            let result = self
                .tool_engine
                .execute(&tc.fn_name, &tc.fn_arguments)
                .await;
            match result {
                ToolResult::Question(title, options) => {
                    let answer =
                        on_event(HarnessEvent::AskUser { title, options }).unwrap_or_default();
                    self.send_tool_result_event(tc, answer, false, on_event);
                }
                ToolResult::Text(text) => {
                    if tc.fn_name == "plan" {
                        let plan_text = RenderUtil::plan_to_md(&tc.fn_arguments);
                        self.last_plan_content = Some(plan_text);
                    }
                    self.send_tool_result_event(tc, text, false, on_event);
                }
                ToolResult::FileEdit { message, diff } => {
                    self.send_diff_view_event(tc, message, diff, on_event);
                }
                ToolResult::Error(err) => {
                    self.send_tool_result_event(tc, err, true, on_event);
                }
            }
        }
    }

    pub fn send_tool_call_event<F>(&mut self, tc: &ToolCall, on_event: &mut F)
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        on_event(HarnessEvent::ToolCall {
            name: tc.fn_name.to_string(),
            arguments: tc.fn_arguments.to_string(),
            start_time: time_now(),
        });
    }

    pub fn send_tool_result_event<F>(
        &mut self,
        tc: &ToolCall,
        result: String,
        error: bool,
        on_event: &mut F,
    ) where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let mut message: ChatMessage = ToolResponse::new(&tc.call_id, result.clone()).into();
        message.content.push(ContentPart::Custom(CustomPart {
            model_iden: None,
            data: json!({ "tool_error": error }),
        }));
        self.history.messages.push(message);
        on_event(HarnessEvent::ToolResult {
            name: tc.fn_name.to_string(),
            result,
            error,
        });
    }

    fn send_diff_view_event<F>(
        &mut self,
        tc: &ToolCall,
        result: String,
        diff: DiffView,
        on_event: &mut F,
    ) where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let marker = diff.to_part();
        let mut message: ChatMessage = ToolResponse::new(&tc.call_id, result.clone()).into();
        message.content.push(marker);
        self.history.messages.push(message);
        on_event(HarnessEvent::ToolResult {
            name: tc.fn_name.to_string(),
            result,
            error: false,
        });
        on_event(HarnessEvent::DiffView(diff));
    }
}
