/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    diff_view::DiffLineKind,
    harness::{Harness, HarnessEvent, HarnessSnapshot, UserRequest},
    render::util::RenderUtil,
    token_usage::format_arrows,
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
        self.harness
            .run_turn(UserRequest::Prompt(prompt), |event| match event {
                HarnessEvent::UserPrompt(text) => {
                    println!("user> {}", text);
                    let _ = io::stdout().flush();
                    None
                }
                HarnessEvent::PlanMessage(text) => {
                    println!("plan> {}", text);
                    let _ = io::stdout().flush();
                    None
                }
                HarnessEvent::Thinking(text) => {
                    if !agent_started {
                        print!("agent> ");
                        agent_started = true;
                    }
                    print!("{}", text);
                    let _ = io::stdout().flush();
                    None
                }
                HarnessEvent::AgentMessage(text) => {
                    if !agent_started {
                        print!("agent> ");
                        agent_started = true;
                    }
                    print!("{}", text);
                    let _ = io::stdout().flush();
                    None
                }
                HarnessEvent::SystemMessage(text) => {
                    if !agent_started {
                        print!("agent> ");
                        agent_started = true;
                    }
                    print!("{}", text);
                    let _ = io::stdout().flush();
                    None
                }
                HarnessEvent::CompactStart { start_time: _ } => {
                    println!("{} Compaction {}", "─".repeat(14), "─".repeat(14));
                    let _ = io::stdout().flush();
                    None
                }
                HarnessEvent::CompactSummary(text) => {
                    if !agent_started {
                        print!("agent> ");
                        agent_started = true;
                    }
                    print!("{}", text);
                    let _ = io::stdout().flush();
                    None
                }
                HarnessEvent::ToolCall {
                    name,
                    arguments,
                    start_time: _,
                } => {
                    if agent_started {
                        println!();
                        agent_started = false;
                    }
                    println!("tool-call> {} ({})", name, arguments);
                    None
                }
                HarnessEvent::ToolResult {
                    name,
                    result,
                    error,
                } => {
                    if error {
                        println!("tool-error> {}: {}", name, result);
                    } else {
                        println!("tool-result> {}: {}", name, result);
                    }
                    None
                }
                HarnessEvent::DiffView(diff) => {
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
                HarnessEvent::TokenUsage {
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
                HarnessEvent::AskUser { title, options } => {
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
                HarnessEvent::TurnStarted(_) => None,
                HarnessEvent::QueueChanged(_) => None,
                HarnessEvent::TurnDuration(duration_ms) => {
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
