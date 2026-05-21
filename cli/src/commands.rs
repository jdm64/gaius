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
    input::{Input, InputMode, PickList},
    models::{ModelPickerRow, Models, model_picker_rows},
    session::Session,
    tui::{TerminalGuard, TuiApp, TuiMessage},
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
            InputMode::PromptInput => {
                Input::handle_prompt_input(app, key, guard, harness, config).await?
            }
            InputMode::Command { picker } => {
                Self::handle_command_mode(app, key, picker, harness, config).await
            }
            InputMode::Session { picker } => Self::handle_session_mode(app, key, picker, harness),
            InputMode::SessionRename { picker } => {
                Self::handle_session_rename_mode(app, key, picker)
            }
            InputMode::Models { picker } => {
                Self::handle_models_mode(app, key, picker, harness, config).await
            }
            InputMode::Agents { picker } => Self::handle_agents_mode(app, key, picker, harness),
            InputMode::Question {
                title: _,
                options: _,
                selected: _,
            } => InputMode::PromptInput,
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
                        app.clear_messages();
                        Input::reset_history_scroll(app);
                        app.status = "New session created".to_string();
                        app.context_tokens = None;
                    }
                    Err(e) => {
                        app.status = e.to_string();
                    }
                };
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
                match Models::list(config).await {
                    Ok(models) => {
                        let recent_models = Models::load_recent().unwrap_or_default();
                        let rows = model_picker_rows("", &models, &recent_models);
                        let filtered = Input::filter_model_rows("", &rows);
                        app.status = format!("Loaded {} models", models.len());
                        InputMode::Models {
                            picker: PickList::new(rows, filtered),
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
                Input::clear_input(app);
                app.status = format!("Loaded {} agents", agents.len());
                InputMode::Agents {
                    picker: PickList::all(agents),
                }
            }
            "streaming" => {
                harness.set_streaming(!harness.streaming());
                app.status = format!("Streaming = {}", harness.streaming());
                Input::clear_input(app);
                InputMode::PromptInput
            }
            _ => {
                app.push_message(TuiMessage::SystemMessage(format!(
                    "Unknown command: /{}",
                    command
                )));
                Input::reset_history_scroll(app);
                Input::clear_input(app);
                InputMode::PromptInput
            }
        }
    }

    pub async fn handle_command_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        mut picker: PickList<Command>,
        harness: &mut Harness,
        config: &Config,
    ) -> InputMode {
        Input::handle_input_cursor(app, key);
        match key.code {
            KeyCode::Esc => return InputMode::PromptInput,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return InputMode::Exit;
            }
            KeyCode::Up => {
                picker.move_up();
            }
            KeyCode::Down => {
                picker.move_down();
            }
            KeyCode::Enter if !picker.is_empty() => {
                let command = picker.selected_row().map(|row| row.name);
                if let Some(command) = command {
                    return Self::execute_command(app, harness, config, command).await;
                }
            }
            KeyCode::Backspace | KeyCode::Delete | KeyCode::Char(_) => {
                picker.replace_filter(Input::filter_commands(&app.input, &picker.rows));
                if picker.is_empty() {
                    return InputMode::PromptInput;
                }
            }
            _ => {}
        }

        InputMode::Command { picker }
    }

    pub fn handle_session_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        mut picker: PickList<Session>,
        harness: &mut Harness,
    ) -> InputMode {
        match key.code {
            KeyCode::Esc => return InputMode::PromptInput,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return InputMode::Exit;
            }
            KeyCode::Up if !picker.is_empty() => {
                picker.move_up();
            }
            KeyCode::Down if !picker.is_empty() => {
                picker.move_down();
            }
            KeyCode::Enter if !picker.is_empty() => {
                let Some(session) = picker.selected_row() else {
                    return InputMode::Session { picker };
                };
                if let Some(session_id) = &session.id {
                    match harness.load_session_by_id(session_id) {
                        Ok(()) => {
                            app.clear_messages();
                            Input::reset_history_scroll(app);
                            app.status = format!("Loaded session: {}", session.display_name());
                            app.context_tokens = None;
                            app.load_history(harness);
                            return InputMode::PromptInput;
                        }
                        Err(e) => {
                            app.status = format!("Error loading session: {}", e);
                        }
                    }
                } else {
                    app.status = "Error loading session: missing session id".to_string();
                }
            }
            KeyCode::Char('d')
                if key.modifiers.contains(KeyModifiers::CONTROL) && !picker.is_empty() =>
            {
                let Some(session) = picker.selected_row() else {
                    return InputMode::Session { picker };
                };
                if let Some(session_id) = &session.id {
                    let display_name = session.display_name();
                    if let Err(e) = Session::delete(session_id) {
                        app.status = format!("Error deleting session: {}", e);
                    } else {
                        let sessions = Session::list();
                        let filtered = (0..sessions.len()).collect();
                        picker.replace_rows(sessions, filtered);
                        app.status = format!("Deleted session: {}", display_name);
                    }
                } else {
                    app.status = "Error deleting session: missing session id".to_string();
                }
            }
            KeyCode::Char('e')
                if key.modifiers.contains(KeyModifiers::CONTROL) && !picker.is_empty() =>
            {
                let Some(session) = picker.selected_row() else {
                    return InputMode::Session { picker };
                };
                app.input = session.display_name();
                app.input_cursor = app.input.chars().count();
                app.status = "Rename session".to_string();
                return InputMode::SessionRename { picker };
            }
            _ => {}
        };

        InputMode::Session { picker }
    }

    pub fn handle_session_rename_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        mut picker: PickList<Session>,
    ) -> InputMode {
        match key.code {
            KeyCode::Esc => {
                Input::clear_input(app);
                return InputMode::Session { picker };
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return InputMode::Exit;
            }
            KeyCode::Enter => {
                let new_name = app.input.trim().to_string();
                if new_name.is_empty() {
                    app.status = "Session name cannot be empty".to_string();
                    return InputMode::SessionRename { picker };
                }

                if picker.is_empty() {
                    app.status = "No session selected".to_string();
                    Input::clear_input(app);
                    return InputMode::Session { picker };
                }

                let selected_id = picker.selected_row().and_then(|session| session.id.clone());
                let Some(session) = picker.selected_row_mut() else {
                    app.status = "No session selected".to_string();
                    Input::clear_input(app);
                    return InputMode::Session { picker };
                };
                match session.rename(new_name.clone()) {
                    Ok(()) => {
                        let sessions = Session::list();
                        let filtered = (0..sessions.len()).collect();
                        picker.replace_rows(sessions, filtered);
                        if let Some(selected_id) = selected_id {
                            picker.selected = picker
                                .filtered
                                .iter()
                                .position(|row_index| {
                                    picker.rows[*row_index].id.as_deref()
                                        == Some(selected_id.as_str())
                                })
                                .unwrap_or_else(|| {
                                    picker.selected.min(picker.filtered.len().saturating_sub(1))
                                });
                        }
                        picker.clamp_selected();
                        Input::clear_input(app);
                        app.status = format!("Renamed session: {}", new_name);
                        return InputMode::Session { picker };
                    }
                    Err(e) => {
                        app.status = format!("Error renaming session: {}", e);
                        return InputMode::SessionRename { picker };
                    }
                }
            }
            _ => {
                Input::handle_input_cursor(app, key);
            }
        }

        InputMode::SessionRename { picker }
    }

    pub async fn handle_models_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        mut picker: PickList<ModelPickerRow>,
        harness: &mut Harness,
        config: &Config,
    ) -> InputMode {
        Input::handle_input_cursor(app, key);
        match key.code {
            KeyCode::Esc => return InputMode::PromptInput,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return InputMode::Exit;
            }
            KeyCode::Up => {
                picker.move_up();
            }
            KeyCode::Down => {
                picker.move_down();
            }
            KeyCode::Enter => {
                let Some(ModelPickerRow::Model(selected_model)) = picker.selected_row() else {
                    app.status = "No matching models".to_string();
                    return InputMode::Models { picker };
                };
                match selected_model.create_client(config) {
                    Ok(client) => {
                        let model_id = selected_model.id.clone();
                        harness.set_model(client, model_id.clone());
                        app.model = model_id.clone();
                        let _ = Models::remember_recent_model(selected_model);
                        Input::clear_input(app);
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
                    Ok(reloaded_models) => {
                        let recent_models = Models::load_recent().unwrap_or_default();
                        let rows = model_picker_rows(&app.input, &reloaded_models, &recent_models);
                        let filtered = Input::filter_model_rows(&app.input, &rows);
                        let count = reloaded_models.len();
                        picker.replace_rows(rows, filtered);
                        app.status = format!("Reloaded {} models", count);
                    }
                    Err(err) => {
                        app.status = format!("Error reloading models: {}", err);
                    }
                }
            }
            _ if input_changed_key(key) => {
                picker.replace_filter(Input::filter_model_rows(&app.input, &picker.rows));
            }
            _ => {}
        };

        InputMode::Models { picker }
    }

    pub fn filtered_model_indices(input: &str, rows: &[ModelPickerRow]) -> Vec<usize> {
        Input::filter_model_rows(input, rows)
    }

    pub fn handle_agents_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        mut picker: PickList<AgentDefinition>,
        harness: &mut Harness,
    ) -> InputMode {
        Input::handle_input_cursor(app, key);
        match key.code {
            KeyCode::Esc => {
                return InputMode::PromptInput;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return InputMode::Exit;
            }
            KeyCode::Up => {
                picker.move_up();
            }
            KeyCode::Down => {
                picker.move_down();
            }
            KeyCode::Enter => {
                let Some(selected_agent) = picker.selected_row().cloned() else {
                    app.status = "No matching agents".to_string();
                    return InputMode::Agents { picker };
                };
                harness.set_agent(selected_agent.clone());
                app.agent_name = selected_agent.name.clone();
                Input::clear_input(app);
                app.status = format!("Selected agent: {}", selected_agent.name);
                return InputMode::PromptInput;
            }
            _ if input_changed_key(key) => {
                picker.replace_filter(Input::filter_agents(&app.input, &picker.rows));
            }
            _ => {}
        }

        InputMode::Agents { picker }
    }

    pub fn filtered_agent_indices(input: &str, agents: &[AgentDefinition]) -> Vec<usize> {
        Input::filter_agents(input, agents)
    }
}

pub fn wrap(i: i32, n: usize) -> usize {
    if n > 0 {
        let m = n as i32;
        ((i % m + m) % m) as usize
    } else {
        i as usize
    }
}

fn input_changed_key(key: event::KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Backspace | KeyCode::Delete | KeyCode::Char('u') | KeyCode::Char('k')
    ) && key.modifiers.contains(KeyModifiers::CONTROL)
        || matches!(key.code, KeyCode::Backspace | KeyCode::Delete)
        || matches!(key.code, KeyCode::Char(_))
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
}
