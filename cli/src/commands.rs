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
    config::ProviderConfig,
    harness_actor::HarnessActorHandle,
    input::{Input, InputMode, PickList, ProviderInfoRow},
    models::{ModelPickerRow, Models, model_picker_rows},
    session::Session,
    tui::{TuiApp, TuiMessage},
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
            Command {
                name: "thinking",
                description: "Toggle rendering of thinking messages on/off",
            },
        ]
    }

    pub async fn handle_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        actor: &HarnessActorHandle,
    ) -> Result<(), Box<dyn Error>> {
        let mode = mem::replace(&mut app.mode, InputMode::PromptInput);
        app.mode = match mode {
            InputMode::PromptInput => Input::handle_prompt_input(app, key, actor).await?,
            InputMode::Command { picker } => {
                Self::handle_command_mode(app, key, picker, actor).await
            }
            InputMode::Session { picker } => {
                Self::handle_session_mode(app, key, picker, actor).await
            }
            InputMode::SessionRename { picker } => {
                Self::handle_session_rename_mode(app, key, picker)
            }
            InputMode::Models { picker } => Self::handle_models_mode(app, key, picker, actor).await,
            InputMode::AddProvider { picker } => {
                Self::handle_add_provider_mode(app, key, picker).await
            }
            InputMode::Agents { picker } => Self::handle_agents_mode(app, key, picker, actor).await,
            InputMode::Files { picker } => Input::handle_files_mode(app, key, picker).await,
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
        actor: &HarnessActorHandle,
        command: &str,
    ) -> InputMode {
        match command {
            "new" => {
                if !app.harness_idle() {
                    app.status =
                        "Agent is busy; finish current turn before creating a session".to_string();
                } else {
                    match actor.new_session().await {
                        Ok(snapshot) => {
                            app.apply_snapshot(&snapshot);
                            app.clear_messages();
                            Input::reset_history_scroll(app);
                            app.status = "New session created".to_string();
                            app.context_tokens = None;
                        }
                        Err(e) => {
                            app.status = e;
                        }
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
                Self::build_models_picklist(app).await
            }
            "agents" => {
                let agents = app.config.agents().all().to_vec();
                Input::clear_input(app);
                app.status = format!("Loaded {} agents", agents.len());
                InputMode::Agents {
                    picker: PickList::all(agents),
                }
            }
            "streaming" => {
                if !app.harness_idle() {
                    app.status =
                        "Agent is busy; finish current turn before changing streaming".to_string();
                } else {
                    match actor.toggle_streaming().await {
                        Ok(snapshot) => {
                            app.apply_snapshot(&snapshot);
                            app.status = format!("Streaming = {}", snapshot.streaming);
                        }
                        Err(err) => app.status = err,
                    }
                }
                Input::clear_input(app);
                InputMode::PromptInput
            }
            "thinking" => {
                app.toggle_thinking();
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
        actor: &HarnessActorHandle,
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
                    return Self::execute_command(app, actor, command).await;
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

    pub async fn handle_session_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        mut picker: PickList<Session>,
        actor: &HarnessActorHandle,
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
                    if !app.harness_idle() {
                        app.status = "Agent is busy; finish current turn before loading a session"
                            .to_string();
                    } else {
                        match actor.load_session(session_id.clone()).await {
                            Ok(snapshot) => {
                                app.apply_snapshot(&snapshot);
                                app.clear_messages();
                                Input::reset_history_scroll(app);
                                app.status = format!("Loaded session: {}", session.display_name());
                                app.context_tokens = None;
                                match actor.replay_history().await {
                                    Ok(snapshot) => app.apply_snapshot(&snapshot),
                                    Err(err) => app.status = err,
                                }
                                return InputMode::PromptInput;
                            }
                            Err(e) => {
                                app.status = format!("Error loading session: {}", e);
                            }
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
        actor: &HarnessActorHandle,
    ) -> InputMode {
        Input::handle_input_cursor(app, key);
        match key.code {
            KeyCode::Esc => return InputMode::PromptInput,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return InputMode::Exit;
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Input::clear_input(app);
                app.status = "Add provider".to_string();
                let rows = vec![
                    ProviderInfoRow::Name(String::new()),
                    ProviderInfoRow::Url(String::new()),
                    ProviderInfoRow::Kind("openai".to_string()),
                    ProviderInfoRow::Key(String::new()),
                ];
                let picker = PickList::all(rows);
                return InputMode::AddProvider { picker };
            }
            KeyCode::Up => {
                picker.move_up();
            }
            KeyCode::Down => {
                picker.move_down();
            }
            KeyCode::Enter => {
                let Some(selected_model) = picker.selected_row().and_then(|row| match row {
                    ModelPickerRow::Model(model) | ModelPickerRow::RecentModel(model) => {
                        Some(model)
                    }
                    ModelPickerRow::Header(_) | ModelPickerRow::Separator => None,
                }) else {
                    app.status = "No matching models".to_string();
                    return InputMode::Models { picker };
                };
                match selected_model.create_client(&app.config) {
                    Ok(client) => {
                        if !app.harness_idle() {
                            app.status =
                                "Agent is busy; finish current turn before changing models"
                                    .to_string();
                            return InputMode::Models { picker };
                        }
                        let model_id = selected_model.id.clone();
                        match actor.set_model(client, model_id.clone()).await {
                            Ok(snapshot) => {
                                app.apply_snapshot(&snapshot);
                                app.model = model_id.clone();
                                let _ = Models::remember_recent_model(selected_model);
                                Input::clear_input(app);
                                app.status = format!("Selected model: {}", model_id);
                                return InputMode::PromptInput;
                            }
                            Err(err) => app.status = err,
                        }
                    }
                    Err(err) => {
                        app.status = format!("Error selecting model: {}", err);
                    }
                }
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let Some(ModelPickerRow::RecentModel(model)) = picker.selected_row().cloned()
                else {
                    app.status = "Can only delete recent models".to_string();
                    return InputMode::Models { picker };
                };
                match Models::forget_recent_model(&model) {
                    Ok(updated_recent) => {
                        if let Ok(models) = Models::list(&app.config).await {
                            let rows = model_picker_rows(&app.input, &models, &updated_recent);
                            let filtered = Input::filter_model_rows(&app.input, &rows);
                            picker.replace_rows(rows, filtered);
                        }
                        app.status = format!("Removed from recent models: {}", model.id);
                    }
                    Err(err) => {
                        app.status = format!("Error removing recent model: {}", err);
                    }
                }
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.status = "Reloading models...".to_string();
                match Models::reload(&app.config).await {
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

    pub async fn handle_add_provider_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        mut picker: PickList<ProviderInfoRow>,
    ) -> InputMode {
        match key.code {
            KeyCode::Esc => {
                Input::clear_input(app);
                let result = Self::build_models_picklist(app).await;
                app.status = "Add provider cancelled".to_string();
                return result;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return InputMode::Exit;
            }
            KeyCode::Up => {
                picker.store_input(app);
                picker.move_up();
                picker.load_input(app);
            }
            KeyCode::Down => {
                picker.store_input(app);
                picker.move_down();
                picker.load_input(app);
            }
            KeyCode::Enter => {
                picker.store_input(app);

                let mut name = String::new();
                let mut url = String::new();
                let mut kind = String::new();
                let mut provider_key = String::new();

                for row in &picker.rows {
                    match row {
                        ProviderInfoRow::Name(v) => name = v.clone(),
                        ProviderInfoRow::Url(v) => url = v.clone(),
                        ProviderInfoRow::Kind(v) => kind = v.clone(),
                        ProviderInfoRow::Key(v) => provider_key = v.clone(),
                    }
                }

                let provider = ProviderConfig {
                    name: name.trim().to_string(),
                    url: url.trim().to_string(),
                    kind: kind.trim().to_string(),
                    key: provider_key.trim().to_string(),
                };

                if let Err(err) = app.config.validate_provider_config(&provider) {
                    app.status = format!("Invalid provider: {}", err);
                    return InputMode::AddProvider { picker };
                }

                app.status = "Validating provider...".to_string();
                match Models::list_models(&provider).await {
                    Ok(_) => match app.config.add_provider(provider) {
                        Ok(()) => match Models::reload(&app.config).await {
                            Ok(reloaded_models) => {
                                let recent_models = Models::load_recent().unwrap_or_default();
                                let rows = model_picker_rows("", &reloaded_models, &recent_models);
                                let filtered = Input::filter_model_rows("", &rows);
                                Input::clear_input(app);
                                app.status = format!(
                                    "Added provider; loaded {} models",
                                    reloaded_models.len()
                                );
                                return InputMode::Models {
                                    picker: PickList::new(rows, filtered),
                                };
                            }
                            Err(err) => {
                                app.status =
                                    format!("Added provider, but failed to reload models: {}", err);
                                Input::clear_input(app);
                                return InputMode::PromptInput;
                            }
                        },
                        Err(err) => {
                            app.status = format!("Error adding provider: {}", err);
                        }
                    },
                    Err(err) => {
                        app.status = format!("Provider validation failed: {}", err);
                    }
                }
            }
            _ => {
                Input::handle_input_cursor(app, key);
            }
        }

        InputMode::AddProvider { picker }
    }

    async fn build_models_picklist(app: &mut TuiApp) -> InputMode {
        match Models::list(&app.config).await {
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

    pub fn filtered_model_indices(input: &str, rows: &[ModelPickerRow]) -> Vec<usize> {
        Input::filter_model_rows(input, rows)
    }

    pub async fn handle_agents_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        mut picker: PickList<AgentDefinition>,
        actor: &HarnessActorHandle,
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
                if !app.harness_idle() {
                    app.status =
                        "Agent is busy; finish current turn before changing agents".to_string();
                    return InputMode::Agents { picker };
                }
                match actor.set_agent(selected_agent.clone()).await {
                    Ok(snapshot) => {
                        app.apply_snapshot(&snapshot);
                        app.agent_name = selected_agent.name.clone();
                        Input::clear_input(app);
                        app.status = format!("Selected agent: {}", selected_agent.name);
                        return InputMode::PromptInput;
                    }
                    Err(err) => app.status = err,
                }
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

pub fn input_changed_key(key: event::KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Backspace | KeyCode::Delete | KeyCode::Char('u') | KeyCode::Char('k')
    ) && key.modifiers.contains(KeyModifiers::CONTROL)
        || matches!(key.code, KeyCode::Backspace | KeyCode::Delete)
        || matches!(key.code, KeyCode::Char(_))
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
}
