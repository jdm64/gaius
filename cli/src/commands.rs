/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::Agents,
    dirs::Dirs,
    harness_actor::HarnessActorHandle,
    input::{Input, InputMode, picklist::PickList},
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
                        Input::scroll_history_bottom(app);
                        app.status = "New session created".to_string();
                        app.context_tokens = None;
                    }
                    Err(e) => app.status = e,
                }
                Input::clear_input(app);
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
                        app.status = "Forked session".to_string();
                        Input::scroll_history_bottom(app);
                    }
                    Err(e) => app.status = e,
                }
                Input::clear_input(app);
                InputMode::PromptInput
            }
            "sessions" => {
                let sessions = Session::list();
                Input::clear_input(app);
                InputMode::Session {
                    picker: PickList::all(sessions),
                }
            }
            "models" => {
                Input::clear_input(app);
                app.status = "Loading models...".to_string();
                InputMode::build_models(app).await
            }
            "agents" => {
                let agents = app.config.agents().all().to_vec();
                Input::clear_input(app);
                app.status = format!("Loaded {} agents", agents.len());
                InputMode::Agents {
                    picker: PickList::all(agents),
                }
            }
            "skills" => {
                let skills = match actor.get_skills().await {
                    Ok(skills) => skills,
                    Err(err) => {
                        app.status = format!("Error loading skills: {}", err);
                        Vec::new()
                    }
                };
                Input::clear_input(app);
                InputMode::Skills {
                    picker: PickList::all(skills),
                }
            }
            "rebuild" => {
                if let Some(busy) = Self::busy_status(app, "rebuilding") {
                    app.status = busy;
                } else {
                    // Reload agents from disk
                    let agents = match Dirs::config_dir() {
                        Ok(config_dir) => match Agents::load(&config_dir) {
                            Ok(agents) => {
                                app.agents = agents;
                                app.agents.all().to_vec()
                            }
                            Err(e) => {
                                app.status = format!("Error reloading agents: {}", e);
                                Input::clear_input(app);
                                return InputMode::PromptInput;
                            }
                        },
                        Err(e) => {
                            app.status = format!("Error getting config dir: {}", e);
                            Input::clear_input(app);
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
                            app.status = "Rebuilt agent and system prompt".to_string();
                        }
                        Err(err) => app.status = err,
                    }
                }
                Input::clear_input(app);
                InputMode::PromptInput
            }
            "compact" => {
                if let Some(busy) = Self::busy_status(app, "compacting") {
                    app.status = busy;
                } else {
                    app.actor_busy = true;
                    app.status = "Compacting conversation...".to_string();
                    Input::scroll_history_bottom(app);
                    if let Err(err) = actor.compact().await {
                        app.status = err;
                    }
                    app.actor_busy = false;
                }
                Input::clear_input(app);
                InputMode::PromptInput
            }
            "info" => match actor.info().await {
                Ok(info) => {
                    Input::clear_input(app);
                    InputMode::SessionInfo { info }
                }
                Err(e) => {
                    app.status = format!("Error getting session info: {}", e);
                    Input::clear_input(app);
                    InputMode::PromptInput
                }
            },
            "streaming" => {
                match Self::when_idle(app, "changing streaming", actor.toggle_streaming()).await {
                    Ok(snapshot) => {
                        app.save_snapshot(&snapshot);
                        app.status = format!("Streaming = {}", snapshot.streaming);
                    }
                    Err(e) => app.status = e,
                }
                Input::clear_input(app);
                InputMode::PromptInput
            }
            "thinking" => {
                app.toggle_thinking();
                Input::clear_input(app);
                InputMode::PromptInput
            }
            "reasoning" => {
                let efforts: Vec<ReasoningEffort> = ReasoningEffort::all().to_vec();
                let filtered = (0..efforts.len()).collect();
                Input::clear_input(app);
                InputMode::Reasoning {
                    picker: PickList::new(efforts, filtered),
                }
            }
            "show-tokens" => {
                app.toggle_token_info();
                Input::clear_input(app);
                InputMode::PromptInput
            }
            "show-diff" => {
                app.toggle_diff_view();
                Input::clear_input(app);
                InputMode::PromptInput
            }
            "plan" => {
                match Self::when_idle(app, "changing plan mode", actor.toggle_plan_mode()).await {
                    Ok(snapshot) => {
                        app.save_snapshot(&snapshot);
                        app.status = format!("Plan mode = {}", snapshot.plan_mode_on);
                    }
                    Err(e) => app.status = e,
                }
                Input::clear_input(app);
                InputMode::PromptInput
            }
            _ => {
                app.push_message(TuiMessage::SystemMessage(format!(
                    "Unknown command: /{}",
                    command
                )));
                Input::clear_input(app);
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
