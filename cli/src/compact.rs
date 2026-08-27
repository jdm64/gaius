/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    harness::{Harness, HarnessEvent, time_now},
    token_usage::TokenUsageLedger,
};
use genai::chat::{ChatMessage, ChatRequest, ChatRole, ContentPart, CustomPart, MessageContent};
use serde_json::json;
use std::error::Error;

const COMPACT_THRESHOLD: f64 = 0.70;
const COMPACT_TAIL_TOKENS: i32 = 24_000;
const DEFAULT_CONTEXT_LEN: i32 = 128_000;

const COMPACT_SYS_PROMPT: &str = "\
You are compacting the context of a coding agent session. Summarize the conversation above \
into handoff notes that let the agent continue the work without losing anything important.

Include:
- the user's requests, goals and any constraints they stated
- decisions made and the reasoning behind them
- files that were read, created or edited, with their paths
- commands run, errors hit and how they were resolved
- unfinished work and the exact next steps

Be factual and concise. Keep file paths, identifiers, names and values exactly as they appear. \
Do not continue the conversation and do not call tools. Reply with the summary only.";

const COMPACT_USER_MESSAGE: &str = "Summarize the conversation above into the handoff notes \
described in your instructions.";

const COMPACT_SUMMARY_HEADER: &str = "The earlier part of this conversation was compacted into \
the following summary to free up context. Continue from where it leaves off.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactOutcome {
    Compacted,
    NothingToCompact,
    Failed,
    Cancelled,
}

pub struct Compact;

impl Compact {
    pub async fn maybe_compact<F>(
        harness: &mut Harness,
        on_event: &mut F,
    ) -> Result<CompactOutcome, Box<dyn Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        if Self::should_compact(harness) {
            Self::compact_now(harness, on_event).await
        } else {
            Ok(CompactOutcome::NothingToCompact)
        }
    }

    pub async fn compact_now<F>(
        harness: &mut Harness,
        on_event: &mut F,
    ) -> Result<CompactOutcome, Box<dyn Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let Some(split) = Self::split_point(&harness.history().messages, harness.token_usage())
        else {
            return Ok(CompactOutcome::NothingToCompact);
        };

        on_event(HarnessEvent::CompactStart {
            start_time: time_now(),
        });

        let older = harness.history().messages[..split].to_vec();
        let (summary, summary_tokens) = match Self::request_summary(harness, older, on_event).await
        {
            Ok(result) => result,
            Err(err) => {
                if harness.is_cancel() {
                    return Ok(CompactOutcome::Cancelled);
                }
                on_event(HarnessEvent::SystemMessage(format!(
                    "Context compaction failed: {err}"
                )));
                return Ok(CompactOutcome::Failed);
            }
        };

        let compacted = Self::compact_history(&harness.history().messages, split, summary);
        harness.apply_compaction(split, compacted, summary_tokens, on_event)?;

        Ok(CompactOutcome::Compacted)
    }

    fn should_compact(harness: &Harness) -> bool {
        harness.token_usage().total_tokens().is_some_and(|tokens| {
            tokens as f64 >= Self::context_len(harness) as f64 * COMPACT_THRESHOLD
        })
    }

    fn context_len(harness: &Harness) -> i32 {
        match harness.model().context_len {
            Some(context_len) if context_len > 0 => context_len,
            _ => DEFAULT_CONTEXT_LEN,
        }
    }

    pub fn split_point(messages: &[ChatMessage], token_usage: &TokenUsageLedger) -> Option<usize> {
        let mut split = token_usage.tail_start(COMPACT_TAIL_TOKENS)?;
        while split > 0 && messages[split].role == ChatRole::Tool {
            split -= 1;
        }

        (split > 0).then_some(split)
    }

    fn request(messages: Vec<ChatMessage>) -> ChatRequest {
        let mut request = ChatRequest::new(messages);
        request.system = Some(COMPACT_SYS_PROMPT.to_string());
        request
            .messages
            .push(ChatMessage::user(COMPACT_USER_MESSAGE.to_string()));

        request
    }

    /// Run the summary request and return the summary text together with the
    /// tokens it cost to generate, so the cost can be attached to the summary
    /// message that replaces the compacted history.
    async fn request_summary<F>(
        harness: &mut Harness,
        messages: Vec<ChatMessage>,
        on_event: &mut F,
    ) -> Result<(String, Option<i32>), Box<dyn Error>>
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let content = harness
            .exec_chat_cancellable(Self::request(messages))
            .await?;

        let summary = content
            .content
            .joined_texts()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .ok_or("compaction request returned an empty summary")?;

        // The summary is shown as agent output, with the usage of the summary
        // request rendered under it.
        on_event(HarnessEvent::CompactSummary(Self::summary_message_text(
            &summary,
        )));
        harness.record_usage(&content.usage, on_event);

        Ok((summary, content.usage.completion_tokens))
    }

    /// The full text of the summary message: the header explaining the
    /// compaction followed by the summary itself.
    fn summary_message_text(summary: &str) -> String {
        format!("{COMPACT_SUMMARY_HEADER}\n\n{summary}")
    }

    pub fn compact_history(
        messages: &[ChatMessage],
        split: usize,
        summary: String,
    ) -> Vec<ChatMessage> {
        let mut compacted = Vec::with_capacity(messages.len() - split + 1);
        // Marker so replay shows the summary as compaction output instead of
        // a user prompt.
        let marker = ContentPart::Custom(CustomPart {
            model_iden: None,
            data: json!("compact_summary"),
        });
        compacted.push(ChatMessage::user(MessageContent::from_parts(vec![
            marker,
            ContentPart::Text(Self::summary_message_text(&summary)),
        ])));
        compacted.extend(messages[split..].iter().cloned());

        compacted
    }
}
