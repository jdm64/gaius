/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use super::{Harness, HarnessEvent};
use crate::{
    harness::UserRequest,
    harness::compact::{Compact, CompactOutcome},
    plan_hook::PlanHook,
    rate_limit::is_rate_limit_error,
    util::time_now,
};
use futures::StreamExt;
use genai::chat::{ChatMessage, ChatStreamEvent, StreamEnd, ToolCall};
use std::{error::Error, time::Duration};
use tokio::time;

impl Harness {
    pub async fn run_turn<F>(
        &mut self,
        request: UserRequest,
        mut on_event: F,
    ) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let start = time_now();
        self.turn_start = Some(start);
        on_event(HarnessEvent::TurnStarted(start));

        let result = self.run_turn_with_events(request, &mut on_event).await;
        if result.is_err() || self.is_cancel() {
            self.drop_queued_prompts(&mut on_event);
        }

        self.turn_start = None;
        let duration = time_now().saturating_sub(start);
        on_event(HarnessEvent::TurnDuration(duration));

        result
    }

    async fn run_turn_with_events<F>(
        &mut self,
        request: UserRequest,
        mut on_event: F,
    ) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        self.set_cancel(false);
        self.update_session_info();

        match request {
            UserRequest::Prompt(text) => self.send_user_message(text, &mut on_event),
            UserRequest::Skill(name) => self.send_skill_message(name, &mut on_event)?,
        }

        loop {
            if self.is_cancel() {
                self.send_system_message("Request Cancelled".to_string(), &mut on_event);
                return Ok(());
            }

            if let Some(prompt) = self.prompt_queue.pop() {
                on_event(HarnessEvent::QueueChanged(self.prompt_queue.len()));
                self.send_user_message(prompt, &mut on_event);
                continue;
            }

            // A cancelled compaction means the user wants out of the turn —
            // sending the request right after would ignore the cancel.
            if Compact::maybe_compact(self, &mut on_event).await? == CompactOutcome::Cancelled {
                self.send_system_message("Request Cancelled".to_string(), &mut on_event);
                return Ok(());
            }

            let tool_calls = if self.streaming {
                self.send_request_streaming(&mut on_event).await?
            } else {
                self.send_request_waiting(&mut on_event).await?
            };

            self.call_tools(&tool_calls, &mut on_event).await;

            let stop_requested = PlanHook::run(self, &mut on_event);

            self.save_history()?;

            if stop_requested || tool_calls.is_empty() {
                if self.is_cancel() {
                    self.send_system_message("Request Cancelled".to_string(), &mut on_event);
                    return Ok(());
                } else if self.prompt_queue.is_empty() {
                    return Ok(());
                }
            }
        }
    }

    pub(super) async fn send_request_streaming<F>(
        &mut self,
        on_event: &mut F,
    ) -> Result<Vec<ToolCall>, Box<dyn std::error::Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let max_retries = 4;
        let mut delay = 3u64;

        for attempt in 0..=max_retries {
            match self.try_send_request_streaming(on_event).await {
                Ok(tool_calls) => return Ok(tool_calls),
                Err(err) => {
                    let is_rate_limit = is_rate_limit_error(err.as_ref());
                    if !is_rate_limit || attempt >= max_retries {
                        return Err(err);
                    }
                    // fall through to retry below
                }
            }

            let message = format!(
                "Rate limit hit. Retrying in {delay}s (attempt {}/{max_retries})...",
                attempt + 1
            );
            on_event(HarnessEvent::SystemMessage(message));

            tokio::select! {
                _ = time::sleep(Duration::from_secs(delay)) => {}
                _ = self.cancel.notified() => {
                    if self.is_cancel() {
                        return Ok(vec![]);
                    }
                }
            }
            delay *= 3;
        }

        Err(format!("Rate limit exceeded after {} retries", max_retries,).into())
    }

    pub(super) async fn send_request_waiting<F>(
        &mut self,
        on_event: &mut F,
    ) -> Result<Vec<ToolCall>, Box<dyn std::error::Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let prompt_idx = self.history.messages.len();
        let response = self.client.chat(self.history.clone()).await?;

        let full_text = response.content.texts().join("");
        if !full_text.is_empty() {
            on_event(HarnessEvent::AgentMessage(full_text.clone()));
        }

        self.history
            .messages
            .push(ChatMessage::assistant(response.content.clone()));

        let response_idx = self.history.messages.len() - 1;
        self.record_usage(prompt_idx, response_idx, Some(response.usage), on_event);

        Ok(response.content.into_tool_calls())
    }

    async fn try_send_request_streaming<F>(
        &mut self,
        on_event: &mut F,
    ) -> Result<Vec<ToolCall>, Box<dyn std::error::Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let prompt_idx = self.history.messages.len();
        let mut response = self.client.chat_streaming(self.history.clone()).await?;

        let mut stream_end = None;
        let mut emitted_text = false;
        loop {
            tokio::select! {
                event = response.stream.next() => {
                    match event {
                        Some(Ok(event)) => {
                            emitted_text |= Self::handle_stream_event(
                                event, on_event, &mut stream_end,
                            );
                        }
                        Some(Err(err)) => return Err(err.into()),
                        None => break,
                    }
                }
                _ = self.cancel.notified() => {
                    if self.is_cancel() {
                        return Ok(vec![]);
                    }
                }
            }
        }

        let stream_end = stream_end.ok_or("Chat stream ended without an end event")?;
        let content = stream_end.captured_content.unwrap_or_default();

        if !emitted_text {
            let text = content.texts().join("");
            if !text.is_empty() {
                on_event(HarnessEvent::AgentMessage(text));
            }
        }

        self.history.messages.push(
            ChatMessage::assistant(content.clone())
                .with_reasoning_content(stream_end.captured_reasoning_content.clone()),
        );

        let response_idx = self.history.messages.len() - 1;
        self.record_usage(
            prompt_idx,
            response_idx,
            stream_end.captured_usage,
            on_event,
        );

        Ok(content.into_tool_calls())
    }

    fn handle_stream_event<F>(
        event: ChatStreamEvent,
        on_event: &mut F,
        stream_end: &mut Option<StreamEnd>,
    ) -> bool
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        match event {
            ChatStreamEvent::Chunk(chunk) => {
                if !chunk.content.is_empty() {
                    on_event(HarnessEvent::AgentMessage(chunk.content));
                    true
                } else {
                    false
                }
            }
            ChatStreamEvent::ReasoningChunk(chunk) => {
                if !chunk.content.is_empty() {
                    on_event(HarnessEvent::Thinking(chunk.content));
                }
                false
            }
            ChatStreamEvent::ThoughtSignatureChunk(chunk) => {
                if !chunk.content.is_empty() {
                    on_event(HarnessEvent::Thinking(chunk.content));
                }
                false
            }
            ChatStreamEvent::End(end) => {
                *stream_end = Some(end);
                false
            }
            ChatStreamEvent::Start | ChatStreamEvent::ToolCallChunk(_) => false,
        }
    }
}
