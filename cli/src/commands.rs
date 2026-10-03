/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::Agents,
    dirs::Dirs,
    harness_actor::HarnessActorHandle,
    input::{InputMode, picklist::PickList},
    models::ReasoningEffort,
    session::Session,
    tui::{TuiApp, TuiMessage},
};
use std::future::Future;

#[derive(Clone)]
pub struct Command {
    pub name: &'static str,
    pub description: &'static str,
}

impl Command {
    pub fn list() -> Vec<Self> {
        vec![
            /* Common */
            Self {
                name: "new",
                description: "Clear history and create a new session",
            },
            Self {
                name: "sessions",
                description: "Load and delete sessions",
            },
            Self {
                name: "models",
                description: "List and select models",
            },
            Self {
                name: "reasoning",
                description: "Set reasoning effort level",
            },
            /* Prompt */
            Self {
                name: "agents",
                description: "List and select agents",
            },
            Self {
                name: "skills",
                description: "List available skills",
            },
            Self {
                name: "plan",
                description: "Toggle plan mode on/off",
            },
            Self {
                name: "rebuild",
                description: "Reload agents, skills, and AGENTS.md",
            },
            /* Session */
            Self {
                name: "compact",
                description: "Compact conversation history into summary",
            },
            Self {
                name: "fork",
                description: "Copy the current session with a new id",
            },
            Self {
                name: "info",
                description: "Show session info",
            },
            /* Display */
            Self {
                name: "show-thinking",
                description: "Toggle rendering of thinking messages",
            },
            Self {
                name: "show-tokens",
                description: "Toggle rendering of token info messages",
            },
            Self {
                name: "show-diff",
                description: "Toggle rendering of diff messages",
            },
            Self {
                name: "streaming",
                description: "Toggle streaming mode",
            },
        ]
    }

    pub async fn execute(app: &mut TuiApp, actor: &HarnessActorHandle, command: &str) -> InputMode {
        match command {
            "new" => {
                match Self::when_idle(app, "creating a session", actor.new_session()).await {
                    Ok(snapshot) => {
                        app.save_snapshot(&snapshot);
                        app.clear_messages();
                        app.history.scroll_bottom();
                        app.editor.status = "New session created".to_string();
                        app.context_tokens = None;
                    }
                    Err(e) => app.editor.status = e,
                }
                app.editor.clear_input();
                InputMode::PromptInput
            }
            "fork" => {
                match Self::when_idle(app, "forking a session", actor.fork_session()).await {
                    Ok(snapshot) => {
                        app.save_snapshot(&snapshot);
                        app.push_message(TuiMessage::SystemMessage(format!(
                            "Session forked: {}",
                            snapshot.session_id.unwrap_or("<unknown>".to_string()),
                        )));
                        app.editor.status = "Forked session".to_string();
                        app.history.scroll_bottom();
                    }
                    Err(e) => app.editor.status = e,
                }
                app.editor.clear_input();
                InputMode::PromptInput
            }
            "sessions" => {
                let sessions = Session::list();
                app.editor.clear_input();
                InputMode::Session {
                    picker: PickList::all(sessions),
                }
            }
            "models" => {
                app.editor.status_clear_input("Loading models...");
                app.editor.build_models(&app.config).await
            }
            "agents" => {
                let agents = app.config.agents().all().to_vec();
                app.editor
                    .status_clear_input(&format!("Loaded {} agents", agents.len()));
                InputMode::Agents {
                    picker: PickList::all(agents),
                }
            }
            "skills" => {
                let skills = match actor.get_skills().await {
                    Ok(skills) => skills,
                    Err(err) => {
                        app.editor.status = format!("Error loading skills: {}", err);
                        Vec::new()
                    }
                };
                app.editor.clear_input();
                InputMode::Skills {
                    picker: PickList::all(skills),
                }
            }
            "rebuild" => {
                if let Some(busy) = Self::busy_status(app, "rebuilding") {
                    app.editor.status = busy;
                } else {
                    // Reload agents from disk
                    let agents = match Dirs::config_dir() {
                        Ok(config_dir) => match Agents::load(&config_dir) {
                            Ok(agents) => {
                                app.agents = agents;
                                app.agents.all().to_vec()
                            }
                            Err(e) => {
                                app.editor
                                    .status_clear_input(&format!("Error reloading agents: {}", e));
                                return InputMode::PromptInput;
                            }
                        },
                        Err(e) => {
                            app.editor
                                .status_clear_input(&format!("Error getting config dir: {}", e));
                            return InputMode::PromptInput;
                        }
                    };

                    // Find the current agent by name
                    let current_agent_name = &app.snapshot.agent_name;
                    let agent = agents
                        .iter()
                        .find(|a| a.name == *current_agent_name)
                        .cloned()
                        .unwrap_or_else(|| app.agents.default_agent().clone());

                    match actor.rebuild_agent(agent).await {
                        Ok(snapshot) => {
                            app.save_snapshot(&snapshot);
                            app.editor.status = "Rebuilt agent and system prompt".to_string();
                        }
                        Err(err) => app.editor.status = err,
                    }
                }
                app.editor.clear_input();
                InputMode::PromptInput
            }
            "compact" => {
                if let Some(busy) = Self::busy_status(app, "compacting") {
                    app.editor.status = busy;
                } else {
                    app.actor_busy = true;
                    app.editor.status = "Compacting conversation...".to_string();
                    app.history.scroll_bottom();
                    if let Err(err) = actor.compact().await {
                        app.editor.status = err;
                    }
                    app.actor_busy = false;
                }
                app.editor.clear_input();
                InputMode::PromptInput
            }
            "info" => match actor.info().await {
                Ok(info) => {
                    app.editor.clear_input();
                    InputMode::SessionInfo { info }
                }
                Err(e) => {
                    app.editor
                        .status_clear_input(&format!("Error getting session info: {}", e));
                    InputMode::PromptInput
                }
            },
            "streaming" => {
                match Self::when_idle(app, "changing streaming", actor.toggle_streaming()).await {
                    Ok(snapshot) => {
                        app.save_snapshot(&snapshot);
                        app.editor.status = format!("Streaming = {}", snapshot.streaming);
                    }
                    Err(e) => app.editor.status = e,
                }
                app.editor.clear_input();
                InputMode::PromptInput
            }
            "thinking" => {
                app.toggle_thinking();
                app.editor.clear_input();
                InputMode::PromptInput
            }
            "reasoning" => {
                let efforts: Vec<ReasoningEffort> = ReasoningEffort::all().to_vec();
                let filtered = (0..efforts.len()).collect();
                app.editor.clear_input();
                InputMode::Reasoning {
                    picker: PickList::new(efforts, filtered),
                }
            }
            "show-tokens" => {
                app.toggle_token_info();
                app.editor.clear_input();
                InputMode::PromptInput
            }
            "show-diff" => {
                app.toggle_diff_view();
                app.editor.clear_input();
                InputMode::PromptInput
            }
            "plan" => {
                match Self::when_idle(app, "changing plan mode", actor.toggle_plan_mode()).await {
                    Ok(snapshot) => {
                        app.save_snapshot(&snapshot);
                        app.editor.status = format!("Plan mode = {}", snapshot.plan_mode_on);
                    }
                    Err(e) => app.editor.status = e,
                }
                app.editor.clear_input();
                InputMode::PromptInput
            }
            _ => {
                app.push_message(TuiMessage::SystemMessage(format!(
                    "Unknown command: /{}",
                    command
                )));
                app.editor.clear_input();
                InputMode::PromptInput
            }
        }
    }

    pub fn filter_commands(input: &str, commands: &[Command]) -> Vec<usize> {
        let query = input
            .strip_prefix('/')
            .unwrap_or(input)
            .trim()
            .to_lowercase();
        commands
            .iter()
            .enumerate()
            .filter_map(|(index, cmd)| cmd.name.to_lowercase().contains(&query).then_some(index))
            .collect()
    }

    pub fn busy_status(app: &TuiApp, action: &str) -> Option<String> {
        (!app.harness_idle()).then(|| format!("Agent is busy; finish current turn before {action}"))
    }

    pub async fn when_idle<T>(
        app: &TuiApp,
        action: &str,
        fut: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        match Self::busy_status(app, action) {
            Some(status) => Err(status),
            None => fut.await,
        }
    }
}
