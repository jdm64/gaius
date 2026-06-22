/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    config::Config,
    diff_view::DiffLineKind,
    harness::{Harness, HarnessEvent},
    models::Models,
    token_usage::format_arrows,
};
use std::{
    error::Error,
    io::{self, Write},
};

pub struct CliPrompt;

impl CliPrompt {
    /// Run a prompt (or interactive loop) against the given harness.
    ///
    /// If `prompt` is `None`, the user is prompted interactively until they
    /// enter an empty line.
    ///
    /// Returns the session ID, if one was created.
    pub async fn run(
        prompt: Option<String>,
        config: Config,
        harness: &mut Harness,
    ) -> Result<Option<String>, Box<dyn Error>> {
        let first_model = Models::first_from_config(&config).await?;
        harness.set_model(first_model.clone()).await?;
        Self::run_inner(prompt, harness).await?;
        Ok(harness.session_id())
    }

    async fn run_inner(
        prompt: Option<String>,
        harness: &mut Harness,
    ) -> Result<(), Box<dyn Error>> {
        if let Some(prompt) = prompt {
            Self::run_turn(prompt, harness).await?;
        } else {
            loop {
                let input = Self::get_input("user> ")?;
                if input.is_empty() {
                    break;
                }
                Self::run_turn(input, harness).await?;
            }
        }

        Ok(())
    }

    pub async fn run_turn(prompt: String, harness: &mut Harness) -> Result<(), Box<dyn Error>> {
        let mut agent_started = false;
        harness
            .run_turn_with_events(prompt, |event| match event {
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
                HarnessEvent::ToolCall {
                    name,
                    arguments,
                    result,
                    error,
                } => {
                    if agent_started {
                        println!();
                        agent_started = false;
                    }
                    println!("tool-call> {} ({})", name, arguments);
                    if error {
                        println!("tool-error> {}", result);
                    } else {
                        println!("tool-result> {}", result);
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
