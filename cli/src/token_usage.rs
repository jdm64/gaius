/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use genai::chat::Usage;
use serde::{Deserialize, Serialize};

use crate::{harness::HarnessEvent, models::TokenPrice};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TokenUsageSpan {
    pub start: usize,
    pub end: usize,
    pub prompt: Option<i32>,
    pub response: Option<i32>,
}

pub fn format_arrows(prompt: Option<i32>, response: Option<i32>) -> String {
    let in_tok = prompt.map_or("".to_string(), |t| format!("\u{2191}{}", t));
    let out_tok = response.map_or("".to_string(), |t| format!("\u{2193}{}", t));
    format!("{}{}", in_tok, out_tok)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageInfo {
    pub context_turns: Option<i32>,
    pub context_tokens: Option<i32>,
    pub session_turns: Option<i32>,
    pub session_input: Option<i32>,
    pub session_output: Option<i32>,
    pub cost_in: Option<f64>,
    pub cost_read: Option<f64>,
    pub cost_out: Option<f64>,
}

impl UsageInfo {
    pub fn add(&mut self, usage: &Usage, pricing: Option<&TokenPrice>) {
        self.accumulate(usage, pricing, true);
    }

    pub fn add_side(&mut self, usage: &Usage, pricing: Option<&TokenPrice>) {
        self.accumulate(usage, pricing, false);
    }

    fn accumulate(&mut self, usage: &Usage, pricing: Option<&TokenPrice>, in_context: bool) {
        if in_context {
            self.context_tokens =
                Some(usage.prompt_tokens.unwrap_or(0) + usage.completion_tokens.unwrap_or(0));
            *self.context_turns.get_or_insert(0) += 1;
        }
        *self.session_turns.get_or_insert(0) += 1;

        if let Some(prompt_tokens) = usage.prompt_tokens {
            self.session_input = Some(self.session_input.unwrap_or(0) + prompt_tokens);
        }
        if let Some(completion_tokens) = usage.completion_tokens {
            self.session_output = Some(self.session_output.unwrap_or(0) + completion_tokens);
        }

        if let Some(pricing) = pricing {
            let cached = usage
                .prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens)
                .unwrap_or(0);
            let prompt = usage.prompt_tokens.unwrap_or(0);
            let completion = usage.completion_tokens.unwrap_or(0);
            let non_cached = prompt.saturating_sub(cached);

            if let Some(price_in) = pricing.price_in {
                self.cost_in = Some(self.cost_in.unwrap_or(0.0) + (non_cached as f64 * price_in));
            }
            if let Some(price_read) = pricing.price_read {
                self.cost_read = Some(self.cost_read.unwrap_or(0.0) + (cached as f64 * price_read));
            }
            if let Some(price_out) = pricing.price_out {
                self.cost_out =
                    Some(self.cost_out.unwrap_or(0.0) + (completion as f64 * price_out));
            }
        }
    }

    pub fn total_cost(&self) -> Option<f64> {
        let in_cost = self.cost_in.unwrap_or(0.0);
        let read_cost = self.cost_read.unwrap_or(0.0);
        let out_cost = self.cost_out.unwrap_or(0.0);
        let total = in_cost + read_cost + out_cost;
        if total > 0.0 { Some(total) } else { None }
    }

    pub fn clear(&mut self) {
        self.context_tokens = None;
        self.context_turns = None;
    }

    pub fn reset(&mut self) {
        self.clear();
        self.session_input = None;
        self.session_output = None;
        self.session_turns = None;
    }
}

#[derive(Clone)]
pub struct SessionInfo {
    pub id: Option<String>,
    pub usage: UsageInfo,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenUsageLedger {
    pub spans: Vec<TokenUsageSpan>,
    pub last_prompt_tokens: Option<i32>,
    pub last_prompt_index: Option<usize>,
    pub usage: UsageInfo,
}

impl TokenUsageLedger {
    pub fn emit_usage<F>(&self, index: usize, on_event: &mut F)
    where
        F: FnMut(HarnessEvent),
    {
        for span in self.spans_after_message(index) {
            on_event(HarnessEvent::TokenUsage {
                prompt: span.prompt,
                response: span.response,
                total: self.usage.context_tokens,
                cost: self.usage.total_cost(),
            });
        }
    }

    pub fn record_with_event<F>(
        &mut self,
        prompt_index: usize,
        response_index: usize,
        usage: &Usage,
        pricing: Option<&TokenPrice>,
        on_event: &mut F,
    ) where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        for span in self.record(prompt_index, response_index, usage, pricing) {
            on_event(HarnessEvent::TokenUsage {
                prompt: span.prompt,
                response: span.response,
                total: self.usage.context_tokens,
                cost: self.usage.total_cost(),
            });
        }
    }

    pub fn record(
        &mut self,
        prompt_index: usize,
        response_index: usize,
        usage: &Usage,
        pricing: Option<&TokenPrice>,
    ) -> Vec<TokenUsageSpan> {
        let mut added = Vec::new();

        if let Some(prompt_tokens) = usage.prompt_tokens {
            if let (Some(previous_tokens), Some(previous_message_end)) =
                (self.last_prompt_tokens, self.last_prompt_index)
            {
                let prompt_delta = prompt_tokens - previous_tokens;
                if prompt_delta >= 0 && previous_message_end < prompt_index {
                    added.push(TokenUsageSpan {
                        start: previous_message_end,
                        end: prompt_index,
                        prompt: Some(prompt_delta),
                        response: None,
                    });
                }
            } else if prompt_index > 0 {
                added.push(TokenUsageSpan {
                    start: 0,
                    end: prompt_index,
                    prompt: Some(prompt_tokens),
                    response: None,
                });
            }

            self.last_prompt_tokens = Some(prompt_tokens);
            self.last_prompt_index = Some(prompt_index);
        }

        if let Some(completion_tokens) = usage.completion_tokens {
            added.push(TokenUsageSpan {
                start: response_index,
                end: response_index.saturating_add(1),
                prompt: None,
                response: Some(completion_tokens),
            });
        }

        self.spans.extend(added.iter().cloned());
        self.usage.add(usage, pricing);

        added
    }

    pub fn record_side_usage(&mut self, usage: &Usage, pricing: Option<&TokenPrice>) {
        self.usage.add_side(usage, pricing);
    }

    pub fn spans_after_message(
        &self,
        message_index: usize,
    ) -> impl Iterator<Item = &TokenUsageSpan> {
        self.spans
            .iter()
            .filter(move |span| span.end == message_index.saturating_add(1))
    }

    pub fn usage(&self) -> UsageInfo {
        self.usage.clone()
    }

    pub fn total_tokens(&self) -> Option<i32> {
        self.usage.context_tokens
    }

    /// Walk the usage spans backwards until `window_tokens` of the most recent
    /// messages are covered and return the message index the tail starts at.
    ///
    /// Returns `None` when there is no history to keep outside the window,
    /// either because the messages are missing or because the whole history
    /// fits inside the window.
    pub fn tail_start(&self, window_tokens: i32) -> Option<usize> {
        let end = self.spans.last().map_or(0, |i| i.end);
        let mut split = end;
        let mut tokens = 0i32;

        for span in self.spans.iter().rev() {
            // Spans overlapping an earlier one are already accounted for.
            if span.end > split {
                continue;
            }
            split = split.min(span.start);
            tokens += span.prompt.unwrap_or(0).max(0) + span.response.unwrap_or(0).max(0);
            if tokens >= window_tokens {
                break;
            }
        }

        (split > 0 && split < end).then_some(split)
    }

    pub fn compact(&mut self, removed: usize, summary_tokens: Option<i32>) {
        // Compaction replaces the removed prefix with exactly one summary
        // message at index zero.
        let replacement_messages = 1;
        let shift = removed.saturating_sub(replacement_messages);

        let mut spans: Vec<TokenUsageSpan> = self
            .spans
            .iter()
            .filter_map(|span| {
                if span.end <= removed {
                    return None;
                }
                // A span can straddle `removed` — e.g. when the split point
                // was moved back over tool responses — so clip it to the kept
                // messages instead of letting it reach over the summary.
                let start = span.start.max(removed);
                Some(TokenUsageSpan {
                    start: start.saturating_sub(shift),
                    end: span.end.saturating_sub(shift),
                    ..span.clone()
                })
            })
            .collect();

        if let Some(tokens) = summary_tokens {
            // The summary is the first message of the rebuilt history, so it
            // goes in front to keep the spans ordered by message index.
            spans.insert(
                0,
                TokenUsageSpan {
                    start: 0,
                    end: 1,
                    prompt: None,
                    response: Some(tokens),
                },
            );
        }

        self.spans = spans;
        self.last_prompt_tokens = None;
        self.last_prompt_index = None;
        self.usage.context_tokens = Some(
            self.spans
                .iter()
                .map(|span| span.prompt.unwrap_or(0).max(0) + span.response.unwrap_or(0).max(0))
                .sum(),
        );
    }

    /// Reset all fields except cumulative values (accumulated over entire session).
    /// Total turns is reset, but cumulative_turns is preserved.
    pub fn clear_context(&mut self) {
        self.spans.clear();
        self.last_prompt_tokens = None;
        self.last_prompt_index = None;
        self.usage.clear();
    }

    /// Reset all values including cumulative (fresh session).
    pub fn new_context(&mut self) {
        self.spans.clear();
        self.last_prompt_tokens = None;
        self.last_prompt_index = None;
        self.usage.reset();
    }
}
