/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::Agents,
    dirs::Dirs,
    harness_actor::HarnessActorClient,
    input::{
        InputMode,
        picklist::{PickList, PickListFilter},
    },
    models::ReasoningEffort,
    session::Session,
    theme::ColorTheme,
    tui::TuiApp,
};

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
    /* Prompt */
    Agents => "agents", "List and select agents";
    Skills => "skills", "List available skills";
    Plan => "plan", "Toggle plan mode on/off";
    /* Model */
    Models => "models", "List and select models";
    Reasoning => "reasoning", "Set reasoning effort level";
    /* Session */
    Compact => "compact", "Compact conversation history into summary";
    Fork => "fork", "Copy the current session with a new id";
    Info => "info", "Show session info";
    /* Display */
    Theme => "theme", "Select the color theme";
    ShowThinking => "show-thinking", "Toggle rendering of thinking messages";
    ShowTokens => "show-tokens", "Toggle rendering of token info messages";
    ShowDiff => "show-diff", "Toggle rendering of diff messages";
    /* Misc */
    Rebuild => "rebuild", "Reload agents, skills, and AGENTS.md";
    Streaming => "streaming", "Toggle streaming mode";
}

impl PickListFilter for Command {
    fn normalize_query(input: &str) -> String {
        input
            .strip_prefix('/')
            .unwrap_or(input)
            .trim()
            .to_lowercase()
    }

    fn matches_query(&self, query: &str) -> bool {
        self.name().to_lowercase().contains(query)
    }
}

impl Command {
    pub fn list() -> Vec<Self> {
        Self::ALL.to_vec()
    }

    pub async fn execute(
        app: &mut TuiApp,
        actor: &HarnessActorClient,
        command: Command,
    ) -> InputMode {
        match command {
            Self::New => {
                if let Some(busy) = Self::busy_status(app, "creating a session") {
                    app.editor.set_status(busy);
                } else if let Err(err) = actor.new_session().await {
                    app.editor.set_status(err);
                }
                app.editor.clear_input();
                InputMode::PromptInput
            }
            Self::Fork => {
                if let Some(busy) = Self::busy_status(app, "forking a session") {
                    app.editor.set_status(busy);
                } else if let Err(err) = actor.fork_session().await {
                    app.editor.set_status(err);
                }
                app.editor.clear_input();
                InputMode::PromptInput
            }
            Self::Sessions => {
                let sessions = Session::list();
                app.editor.clear_input();
                InputMode::Session {
                    picker: Session::picker_rows(sessions, ""),
                }
            }
            Self::Models => {
                app.editor.status_clear_input("Loading models...");
                app.editor.build_models(&app.config).await
            }
            Self::Agents => {
                let agents = app.config.agents().all().to_vec();
                app.editor
                    .status_clear_input(format!("Loaded {} agents", agents.len()));
                InputMode::Agents {
                    picker: PickList::all(agents),
                }
            }
            Self::Skills => {
                app.editor.status_clear_input("Loading skills...");
                if let Err(err) = actor.get_skills().await {
                    app.editor
                        .set_status(format!("Error loading skills: {}", err));
                }
                InputMode::PromptInput
            }
            Self::Rebuild => {
                if let Some(busy) = Self::busy_status(app, "rebuilding") {
                    app.editor.set_status(busy);
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
                                    .status_clear_input(format!("Error reloading agents: {}", e));
                                return InputMode::PromptInput;
                            }
                        },
                        Err(e) => {
                            app.editor
                                .status_clear_input(format!("Error getting config dir: {}", e));
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

                    if let Err(err) = actor.rebuild_agent(agent).await {
                        app.editor.set_status(err);
                    }
                }
                app.editor.clear_input();
                InputMode::PromptInput
            }
            Self::Compact => {
                if let Some(busy) = Self::busy_status(app, "compacting") {
                    app.editor.set_status(busy);
                } else {
                    app.actor_busy = true;
                    app.editor.set_status("Compacting conversation...");
                    app.history.scroll_bottom();
                    if let Err(err) = actor.compact().await {
                        app.editor.set_status(err);
                    }
                    app.actor_busy = false;
                }
                app.editor.clear_input();
                InputMode::PromptInput
            }
            Self::Info => {
                if let Err(err) = actor.info().await {
                    app.editor
                        .status_clear_input(format!("Error getting session info: {}", err));
                }
                app.editor.clear_input();
                InputMode::PromptInput
            }
            Self::Streaming => {
                if let Some(busy) = Self::busy_status(app, "changing streaming") {
                    app.editor.set_status(busy);
                } else if let Err(err) = actor.toggle_streaming().await {
                    app.editor.set_status(err);
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
                let names: Vec<String> = ColorTheme::names()
                    .iter()
                    .map(|name| name.to_string())
                    .collect();
                let current = app.display_prefs.theme.name().to_string();
                let mut picker = PickList::all(names);
                picker.selected = picker
                    .filtered
                    .iter()
                    .position(|&index| picker.rows[index] == current)
                    .unwrap_or(0);
                InputMode::Theme { picker, current }
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
                if let Some(busy) = Self::busy_status(app, "changing plan mode") {
                    app.editor.set_status(busy);
                } else if let Err(err) = actor.toggle_plan_mode().await {
                    app.editor.set_status(err);
                }
                app.editor.clear_input();
                InputMode::PromptInput
            }
        }
    }

    pub fn busy_status(app: &TuiApp, action: &str) -> Option<String> {
        (!app.harness_idle()).then(|| format!("Agent is busy; finish current turn before {action}"))
    }
}
