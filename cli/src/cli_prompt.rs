/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    diff_view::DiffLineKind,
    harness::{Harness, HarnessMsg, HarnessSnapshot, UserRequest},
    render::util::RenderUtil,
    token_usage::format_arrows,
    tool_engine::ToolStatus,
};
use std::{
    error::Error,
    io::{self, Write},
};

pub struct CliPrompt {
    init_prompt: Option<String>,
    harness: Harness,
}

impl CliPrompt {
    pub fn new(init_prompt: Option<String>, harness: Harness) -> Self {
        CliPrompt {
            init_prompt,
            harness,
        }
    }

    pub async fn run(&mut self) -> Result<HarnessSnapshot, Box<dyn Error>> {
        if let Some(prompt) = self.init_prompt.clone() {
            self.run_turn(prompt).await?;
        } else {
            loop {
                let input = Self::get_input("user> ")?;
                if input.is_empty() {
                    break;
                }
                self.run_turn(input).await?;
            }
        }

        Ok(self.harness.snapshot())
    }

    pub async fn run_turn(&mut self, prompt: String) -> Result<(), Box<dyn Error>> {
        let mut agent_started = false;
        let msg_id = self.harness.next_msg_id();
        let request = UserRequest::Prompt { msg_id, prompt };
        self.harness
            .run_turn(request, |event| match event {
                HarnessMsg::UserPrompt { content: text, .. } => {
                    println!("user> {}", text);
                    let _ = io::stdout().flush();
                    None
                }
                HarnessMsg::PlanMessage { content: text, .. } => {
                    println!("plan> {}", text);
                    let _ = io::stdout().flush();
                    None
                }
                HarnessMsg::Thinking { content: text, .. }
                | HarnessMsg::ThinkingChunk { content: text, .. } => {
                    if !agent_started {
                        print!("agent> ");
                        agent_started = true;
                    }
                    print!("{}", text);
                    let _ = io::stdout().flush();
                    None
                }
                HarnessMsg::AgentMessage { content: text, .. }
                | HarnessMsg::AgentChunk { content: text, .. } => {
                    if !agent_started {
                        print!("agent> ");
                        agent_started = true;
                    }
                    print!("{}", text);
                    let _ = io::stdout().flush();
                    None
                }
                HarnessMsg::SystemMessage(text) => {
                    if !agent_started {
                        print!("agent> ");
                        agent_started = true;
                    }
                    print!("{}", text);
                    let _ = io::stdout().flush();
                    None
                }
                HarnessMsg::CompactStart { .. } => {
                    println!("{} Compaction {}", "─".repeat(14), "─".repeat(14));
                    let _ = io::stdout().flush();
                    None
                }
                HarnessMsg::CompactSummary { content: text, .. } => {
                    if !agent_started {
                        print!("agent> ");
                        agent_started = true;
                    }
                    print!("{}", text);
                    let _ = io::stdout().flush();
                    None
                }
                HarnessMsg::ToolCall {
                    name,
                    arguments,
                    call_id: _,
                    start_time: _,
                } => {
                    if agent_started {
                        println!();
                        agent_started = false;
                    }
                    println!("tool-call> {} ({})", name, arguments);
                    None
                }
                HarnessMsg::ToolResult {
                    name,
                    result,
                    status,
                    ..
                } => {
                    if status != ToolStatus::Valid {
                        println!("tool-error> {}: {}", name, result);
                    } else {
                        println!("tool-result> {}: {}", name, result);
                    }
                    None
                }
                HarnessMsg::DiffView(diff) => {
                    if agent_started {
                        println!();
                        agent_started = false;
                    }
                    println!("diff> {}", diff.file_path);
                    for hunk in diff.hunks {
                        println!(
                            "@@ -{},{} +{},{} @@",
                            hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
                        );
                        for line in hunk.lines {
                            let prefix = match line.kind {
                                DiffLineKind::Context => " ",
                                DiffLineKind::Delete => "-",
                                DiffLineKind::Insert => "+",
                            };
                            println!("{}{}", prefix, line.text);
                            if line.missing_newline {
                                println!("\\ No newline at end of file");
                            }
                        }
                    }
                    None
                }
                HarnessMsg::TokenUsage {
                    prompt,
                    response,
                    total,
                    cost: _,
                } => {
                    if agent_started {
                        println!();
                        agent_started = false;
                    }
                    let net = format_arrows(prompt, response);
                    let total_str = total.unwrap_or_default();
                    println!("tokens> {net} {total_str}");
                    None
                }
                HarnessMsg::AskUser { title, options } => {
                    if agent_started {
                        println!();
                        agent_started = false;
                    }
                    println!("question> {}", title);
                    for (index, option) in options.iter().enumerate() {
                        println!("  {}) {}", index + 1, option);
                    }
                    Self::get_input("answer> ").ok()
                }
                HarnessMsg::TurnStarted(_) => None,
                HarnessMsg::QueueChanged(_) => None,
                HarnessMsg::TurnDuration(duration_ms) => {
                    let time = RenderUtil::format_duration(duration_ms);
                    println!("timing> {}", time);
                    let _ = io::stdout().flush();
                    None
                }
            })
            .await?;

        if agent_started {
            println!();
        }

        Ok(())
    }

    pub fn get_input(label: &str) -> Result<String, Box<dyn Error>> {
        print!("{}", label);
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        Ok(input.trim().to_string())
    }
}
