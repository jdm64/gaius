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

use crate::{
    agents::AgentDefinition,
    config::Config,
    harness::Harness,
    models::{AvailableModel, Models},
    session::Session,
    tui::{InputMode, MessageRole, TerminalGuard, TuiApp, TuiMessage},
};
use crossterm::event::{self, KeyCode, KeyModifiers};
use std::{error::Error, mem};

#[derive(Clone)]
pub struct Command {
    pub name: &'static str,
    pub description: &'static str,
}

pub struct Commands {}

impl Commands {
    pub fn commands() -> Vec<Command> {
        vec![
            Command {
                name: "new",
                description: "Clear history and create a new session",
            },
            Command {
                name: "sessions",
                description: "Load and delete sessions",
            },
            Command {
                name: "models",
                description: "List and select models",
            },
            Command {
                name: "agents",
                description: "List and select agents",
            },
            Command {
                name: "streaming",
                description: "Toggle streaming mode on/off",
            },
        ]
    }

    pub async fn handle_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        guard: &mut TerminalGuard,
        harness: &mut Harness,
        config: &Config,
    ) -> Result<(), Box<dyn Error>> {
        let mode = mem::replace(&mut app.mode, InputMode::PromptInput);
        app.mode = match mode {
            InputMode::PromptInput => app.handle_prompt_input(key, guard, harness, config).await?,
            InputMode::Command { selected, filtered } => {
                Self::handle_command_mode(app, key, selected, filtered, harness, config).await
            }
            InputMode::Session { selected, sessions } => {
                Self::handle_session_mode(app, key, selected, sessions, harness)
            }
            InputMode::Models { selected, models } => {
                Self::handle_models_mode(app, key, selected, models, harness, config).await
            }
            InputMode::Agents { selected, agents } => {
                Self::handle_agents_mode(app, key, selected, agents, harness)
            }
            InputMode::Exit => InputMode::Exit,
        };
        Ok(())
    }

    pub async fn execute_command(
        app: &mut TuiApp,
        harness: &mut Harness,
        config: &Config,
        command: &str,
    ) -> InputMode {
        match command {
            "new" => {
                match harness.new_session() {
                    Ok(_) => {
                        app.messages.clear();
                        app.reset_history_scroll();
                        app.status = "New session created".to_string();
                        app.context_tokens = None;
                    }
                    Err(e) => {
                        app.status = e.to_string();
                    }
                };
                app.clear_input();
                InputMode::PromptInput
            }
            "sessions" => {
                let sessions = Session::list();
                app.clear_input();
                InputMode::Session {
                    selected: 0,
                    sessions,
                }
            }
            "models" => {
                app.clear_input();
                app.status = "Loading models...".to_string();
                match Models::list(config).await {
                    Ok(models) => {
                        app.status = format!("Loaded {} models", models.len());
                        InputMode::Models {
                            selected: 0,
                            models,
                        }
                    }
                    Err(err) => {
                        app.status = format!("Error loading models: {}", err);
                        InputMode::PromptInput
                    }
                }
            }
            "agents" => {
                let agents = config.agents().all().to_vec();
                app.clear_input();
                app.status = format!("Loaded {} agents", agents.len());
                InputMode::Agents {
                    selected: 0,
                    agents,
                }
            }
            "streaming" => {
                harness.set_streaming(!harness.streaming());
                app.status = format!("Streaming = {}", harness.streaming());
                app.clear_input();
                InputMode::PromptInput
            }
            _ => {
                app.messages.push(TuiMessage {
                    role: MessageRole::Agent,
                    text: format!("Unknown command: /{}", command),
                    is_markdown: false,
                });
                app.reset_history_scroll();
                app.clear_input();
                InputMode::PromptInput
            }
        }
    }

    pub async fn handle_command_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        mut selected: usize,
        filtered: Vec<Command>,
        harness: &mut Harness,
        config: &Config,
    ) -> InputMode {
        app.handle_input_cursor(key);
        match key.code {
            KeyCode::Esc => return InputMode::PromptInput,
            KeyCode::Up => {
                selected = wrap(selected as i32 - 1, filtered.len());
            }
            KeyCode::Down => {
                selected = wrap(selected as i32 + 1, filtered.len());
            }
            KeyCode::Enter if !filtered.is_empty() => {
                let command = filtered[selected].name;
                return Self::execute_command(app, harness, config, command).await;
            }
            KeyCode::Backspace | KeyCode::Delete | KeyCode::Char(_) => {
                return app.mode_for_input();
            }
            _ => {}
        }

        InputMode::Command { selected, filtered }
    }

    pub fn handle_session_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        mut selected: usize,
        mut sessions: Vec<String>,
        harness: &mut Harness,
    ) -> InputMode {
        match key.code {
            KeyCode::Esc => return InputMode::PromptInput,
            KeyCode::Up => {
                selected = wrap(selected as i32 - 1, sessions.len());
            }
            KeyCode::Down => {
                selected = wrap(selected as i32 + 1, sessions.len());
            }
            KeyCode::Enter if !sessions.is_empty() => {
                let session_id = &sessions[selected];
                match harness.load_session_by_id(session_id) {
                    Ok(()) => {
                        app.messages.clear();
                        app.reset_history_scroll();
                        app.status = format!("Loaded session: {}", session_id);
                        app.context_tokens = None;
                        app.load_history(harness.history());
                        return InputMode::PromptInput;
                    }
                    Err(e) => {
                        app.status = format!("Error loading session: {}", e);
                    }
                }
            }
            KeyCode::Char('d')
                if key.modifiers.contains(KeyModifiers::CONTROL) && !sessions.is_empty() =>
            {
                let session_id = sessions[selected].clone();
                if let Err(e) = Session::delete(&session_id) {
                    app.status = format!("Error deleting session: {}", e);
                } else {
                    sessions = Session::list();
                    if selected >= sessions.len() && selected > 0 {
                        selected -= 1;
                    }
                    app.status = format!("Deleted session: {}", session_id);
                }
            }
            _ => {}
        };

        InputMode::Session { selected, sessions }
    }

    pub async fn handle_models_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        mut selected: usize,
        models: Vec<AvailableModel>,
        harness: &mut Harness,
        config: &Config,
    ) -> InputMode {
        app.handle_input_cursor(key);
        match key.code {
            KeyCode::Esc => return InputMode::PromptInput,
            KeyCode::Up => {
                let filtered_len = Self::filtered_model_indices(&app.input, &models).len();
                selected = wrap(selected as i32 - 1, filtered_len);
            }
            KeyCode::Down => {
                let filtered_len = Self::filtered_model_indices(&app.input, &models).len();
                selected = wrap(selected as i32 + 1, filtered_len);
            }
            KeyCode::Enter => {
                let filtered = Self::filtered_model_indices(&app.input, &models);
                if filtered.is_empty() {
                    app.status = "No matching models".to_string();
                    return InputMode::Models { selected, models };
                }

                selected = selected.min(filtered.len().saturating_sub(1));
                let selected_model = &models[filtered[selected]];
                match selected_model.create_client(config) {
                    Ok(client) => {
                        let model_id = selected_model.id.clone();
                        harness.set_model(client, model_id.clone());
                        app.model = model_id.clone();
                        app.clear_input();
                        app.status = format!("Selected model: {}", model_id);
                        return InputMode::PromptInput;
                    }
                    Err(err) => {
                        app.status = format!("Error selecting model: {}", err);
                    }
                }
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.status = "Reloading models...".to_string();
                match Models::reload(config).await {
                    Ok(models) => {
                        selected = Self::clamp_model_selection(&app.input, selected, &models);
                        app.status = format!("Reloaded {} models", models.len());
                    }
                    Err(err) => {
                        app.status = format!("Error reloading models: {}", err);
                    }
                }
            }
            KeyCode::Backspace => {
                selected = Self::clamp_model_selection(&app.input, selected, &models);
            }
            KeyCode::Delete => {
                selected = Self::clamp_model_selection(&app.input, selected, &models);
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                selected = Self::clamp_model_selection(&app.input, selected, &models);
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                selected = Self::clamp_model_selection(&app.input, selected, &models);
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                selected = Self::clamp_model_selection(&app.input, selected, &models);
            }
            _ => {}
        };

        InputMode::Models { selected, models }
    }

    pub fn filtered_model_indices(input: &str, models: &[AvailableModel]) -> Vec<usize> {
        let query = input.trim().to_lowercase();
        models
            .iter()
            .enumerate()
            .filter_map(|(index, model)| {
                let is_match = query.is_empty() || model.id.to_lowercase().contains(&query);
                is_match.then_some(index)
            })
            .collect()
    }

    fn clamp_model_selection(input: &str, selected: usize, models: &[AvailableModel]) -> usize {
        let filtered_len = Self::filtered_model_indices(input, models).len();
        selected.min(filtered_len.saturating_sub(1))
    }

    pub fn handle_agents_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        mut selected: usize,
        agents: Vec<AgentDefinition>,
        harness: &mut Harness,
    ) -> InputMode {
        app.handle_input_cursor(key);
        match key.code {
            KeyCode::Esc => {
                return InputMode::PromptInput;
            }
            KeyCode::Up => {
                let filtered_len = Self::filtered_agent_indices(&app.input, &agents).len();
                selected = wrap(selected as i32 - 1, filtered_len);
            }
            KeyCode::Down => {
                let filtered_len = Self::filtered_agent_indices(&app.input, &agents).len();
                selected = wrap(selected as i32 + 1, filtered_len);
            }
            KeyCode::Enter => {
                let filtered = Self::filtered_agent_indices(&app.input, &agents);
                if filtered.is_empty() {
                    app.status = "No matching agents".to_string();
                    return InputMode::Agents { selected, agents };
                }

                selected = selected.min(filtered.len().saturating_sub(1));
                let selected_agent = agents[filtered[selected]].clone();
                harness.set_agent(selected_agent.clone());
                app.agent_name = selected_agent.name.clone();
                app.clear_input();
                app.status = format!("Selected agent: {}", selected_agent.name);
                return InputMode::PromptInput;
            }
            KeyCode::Backspace => {
                selected = Self::clamp_agent_selection(&app.input, selected, &agents);
            }
            KeyCode::Delete => {
                selected = Self::clamp_agent_selection(&app.input, selected, &agents);
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                selected = Self::clamp_agent_selection(&app.input, selected, &agents);
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                selected = Self::clamp_agent_selection(&app.input, selected, &agents);
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                selected = Self::clamp_agent_selection(&app.input, selected, &agents);
            }
            _ => {}
        };

        InputMode::Agents { selected, agents }
    }

    pub fn filtered_agent_indices(input: &str, agents: &[AgentDefinition]) -> Vec<usize> {
        let query = input.trim().to_lowercase();
        agents
            .iter()
            .enumerate()
            .filter_map(|(index, agent)| {
                let is_match = query.is_empty() || agent.name.to_lowercase().contains(&query);
                is_match.then_some(index)
            })
            .collect()
    }

    fn clamp_agent_selection(input: &str, selected: usize, agents: &[AgentDefinition]) -> usize {
        let filtered_len = Self::filtered_agent_indices(input, agents).len();
        selected.min(filtered_len.saturating_sub(1))
    }
}

fn wrap(i: i32, n: usize) -> usize {
    let m = n as i32;
    ((i % m + m) % m) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_behaves_correctly() {
        // Basic wrapping within bounds
        assert_eq!(wrap(0, 5), 0);
        assert_eq!(wrap(2, 5), 2);
        assert_eq!(wrap(4, 5), 4);

        // Wrapping around at boundaries
        assert_eq!(wrap(5, 5), 0);
        assert_eq!(wrap(6, 5), 1);
        assert_eq!(wrap(9, 5), 4);

        // Negative indices wrap to end
        assert_eq!(wrap(-1, 5), 4);
        assert_eq!(wrap(-2, 5), 3);
        assert_eq!(wrap(-5, 5), 0);
        assert_eq!(wrap(-6, 5), 4);

        // Large numbers wrap correctly
        assert_eq!(wrap(12, 5), 2);
        assert_eq!(wrap(20, 7), 6);
        assert_eq!(wrap(100, 10), 0);

        // Edge case: single element
        assert_eq!(wrap(0, 1), 0);
        assert_eq!(wrap(10, 1), 0);
        assert_eq!(wrap(-1, 1), 0);
    }
}
