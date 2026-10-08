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
    theme::ColorTheme,
    tui::{TuiApp, TuiMessage},
};
use std::future::Future;

macro_rules! define_commands {
    ($($variant:ident => $name:literal, $description:literal;)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Command {
            $($variant),*
        }

        impl std::fmt::Display for Command {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.name())
            }
        }

        impl Command {
            pub const ALL: &'static [Command] = &[$(Command::$variant),*];

            pub fn name(self) -> &'static str {
                match self {
                    $(Command::$variant => $name),*
                }
            }

            pub fn description(self) -> &'static str {
                match self {
                    $(Command::$variant => $description),*
                }
            }

            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|cmd| cmd.name() == name)
            }
        }
    };
}

define_commands! {
    /* Common */
    New => "new", "Clear history and create a new session";
    Sessions => "sessions", "Load and delete sessions";
    Models => "models", "List and select models";
    Reasoning => "reasoning", "Set reasoning effort level";
    /* Prompt */
    Agents => "agents", "List and select agents";
    Skills => "skills", "List available skills";
    Plan => "plan", "Toggle plan mode on/off";
    Rebuild => "rebuild", "Reload agents, skills, and AGENTS.md";
    /* Session */
    Compact => "compact", "Compact conversation history into summary";
    Fork => "fork", "Copy the current session with a new id";
    Info => "info", "Show session info";
    /* Display */
    Theme => "theme", "Select the color theme";
    ShowThinking => "show-thinking", "Toggle rendering of thinking messages";
    ShowTokens => "show-tokens", "Toggle rendering of token info messages";
    ShowDiff => "show-diff", "Toggle rendering of diff messages";
    Streaming => "streaming", "Toggle streaming mode";
}

impl Command {
    pub fn list() -> Vec<Self> {
        Self::ALL.to_vec()
    }

    pub async fn execute(
        app: &mut TuiApp,
        actor: &HarnessActorHandle,
        command: Command,
    ) -> InputMode {
        match command {
            Self::New => {
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
            Self::Fork => {
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
            Self::Sessions => {
                let sessions = Session::list();
                app.editor.clear_input();
                InputMode::Session {
                    picker: PickList::all(sessions),
                }
            }
            Self::Models => {
                app.editor.status_clear_input("Loading models...");
                app.editor.build_models(&app.config).await
            }
            Self::Agents => {
                let agents = app.config.agents().all().to_vec();
                app.editor
                    .status_clear_input(&format!("Loaded {} agents", agents.len()));
                InputMode::Agents {
                    picker: PickList::all(agents),
                }
            }
            Self::Skills => {
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
            Self::Rebuild => {
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
            Self::Compact => {
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
            Self::Info => match actor.info().await {
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
            Self::Streaming => {
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
            Self::ShowThinking => {
                app.toggle_thinking();
                app.editor.clear_input();
                InputMode::PromptInput
            }
            Self::Theme => {
                app.editor.clear_input();
                InputMode::Theme {
                    picker: PickList::all(
                        ColorTheme::names().iter().map(|n| n.to_string()).collect(),
                    ),
                }
            }
            Self::Reasoning => {
                let efforts: Vec<ReasoningEffort> = ReasoningEffort::all().to_vec();
                let filtered = (0..efforts.len()).collect();
                app.editor.clear_input();
                InputMode::Reasoning {
                    picker: PickList::new(efforts, filtered),
                }
            }
            Self::ShowTokens => {
                app.toggle_token_info();
                app.editor.clear_input();
                InputMode::PromptInput
            }
            Self::ShowDiff => {
                app.toggle_diff_view();
                app.editor.clear_input();
                InputMode::PromptInput
            }
            Self::Plan => {
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
            .filter_map(|(index, cmd)| cmd.name().to_lowercase().contains(&query).then_some(index))
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
