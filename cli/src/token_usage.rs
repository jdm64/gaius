/* Copyright 2026 Justin Madru (justin.jdm64@gmail.com)
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use genai::chat::Usage;
use serde::{Deserialize, Serialize};

use crate::harness::HarnessEvent;

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
pub struct TokenUsageLedger {
    pub spans: Vec<TokenUsageSpan>,
    last_prompt_tokens: Option<i32>,
    last_prompt_index: Option<usize>,
    last_total_tokens: Option<i32>,
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
                total: self.last_total_tokens,
            });
        }
    }

    pub fn record_with_event<F>(
        &mut self,
        prompt_index: usize,
        response_index: usize,
        usage: &Usage,
        on_event: &mut F,
    ) where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        for span in self.record(prompt_index, response_index, usage) {
            on_event(HarnessEvent::TokenUsage {
                prompt: span.prompt,
                response: span.response,
                total: self.last_total_tokens,
            });
        }
    }

    pub fn record(
        &mut self,
        prompt_index: usize,
        response_index: usize,
        usage: &Usage,
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
        self.last_total_tokens =
            Some(usage.prompt_tokens.unwrap_or(0) + usage.completion_tokens.unwrap_or(0));

        added
    }

    pub fn spans_after_message(
        &self,
        message_index: usize,
    ) -> impl Iterator<Item = &TokenUsageSpan> {
        self.spans
            .iter()
            .filter(move |span| span.end == message_index.saturating_add(1))
    }

    pub fn total_tokens(&self) -> Option<i32> {
        self.last_total_tokens
    }
}
