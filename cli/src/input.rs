/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

pub mod agent;
pub mod file;
pub mod model;
pub mod picklist;
pub mod session;

use crate::{
    agents::AgentDefinition,
    commands::Command,
    harness_actor::HarnessActorHandle,
    input::{file::FileEntry, model::ProviderInfoRow, picklist::PickList},
    models::{ModelPickerRow, ReasoningEffort},
    session::Session,
    skills::Skill,
    token_usage::SessionInfo,
    tui::TuiApp,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::error::Error;
use std::mem;

pub enum InputMode {
    Exit,
    PromptInput,
    Command {
        picker: PickList<Command>,
    },
    Session {
        picker: PickList<Session>,
    },
    SessionRename {
        picker: PickList<Session>,
    },
    Models {
        picker: PickList<ModelPickerRow>,
    },
    AddProvider {
        picker: PickList<ProviderInfoRow>,
    },
    Agents {
        picker: PickList<AgentDefinition>,
    },
    Files {
        picker: PickList<FileEntry>,
    },
    Skills {
        picker: PickList<Skill>,
    },
    Question {
        title: String,
        options: Vec<String>,
        selected: usize,
    },
    SessionInfo {
        info: SessionInfo,
    },
    Reasoning {
        picker: PickList<ReasoningEffort>,
    },
}

impl InputMode {
    pub async fn handle_mode(
        app: &mut TuiApp,
        key: KeyEvent,
        actor: &HarnessActorHandle,
    ) -> Result<(), Box<dyn Error>> {
        let mode = mem::replace(&mut app.mode, Self::PromptInput);
        app.mode = match mode {
            Self::PromptInput => Self::handle_prompt_input(app, key, actor).await?,
            Self::Command { picker } => Self::handle_command(app, key, picker, actor).await,
            Self::Session { picker } => Self::handle_session(app, key, picker, actor).await,
            Self::SessionRename { picker } => Self::handle_session_rename(app, key, picker),
            Self::Models { picker } => Self::handle_models(app, key, picker, actor).await,
            Self::AddProvider { picker } => Self::handle_provider_add(app, key, picker).await,
            Self::Agents { picker } => Self::handle_agents(app, key, picker, actor).await,
            Self::Files { picker } => Self::handle_files(app, key, picker).await,
            Self::Skills { picker } => Self::handle_skills(app, key, picker, actor).await,
            Self::Reasoning { picker } => Self::handle_reasoning(app, key, picker, actor).await,
            Self::Question {
                title: _,
                options: _,
                selected: _,
            } => Self::PromptInput,
            Self::SessionInfo { info } => Self::handle_session_info(key, info),
            Self::Exit => Self::Exit,
        };
        Ok(())
    }

    pub async fn handle_prompt_input(
        app: &mut TuiApp,
        key: KeyEvent,
        actor: &HarnessActorHandle,
    ) -> Result<Self, Box<dyn Error>> {
        Input::handle_input_cursor(app, key);
        match key.code {
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.status = "Cancelling agent...".to_string();
                actor.cancel().await?;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(Self::Exit);
            }
            KeyCode::Backspace | KeyCode::Delete => {
                return Ok(Self::mode_for_input(app));
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(Self::mode_for_input(app));
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(Self::mode_for_input(app));
            }
            KeyCode::Up if app.input_cursor == 0 => {
                let len = app.prompt_history.len();
                if len > 0 {
                    app.prompt_history_idx = match app.prompt_history_idx {
                        None => Some(0),
                        Some(i) if i + 1 < len => Some(i + 1),
                        Some(i) => Some(i),
                    };
                    if let Some(idx) = app.prompt_history_idx {
                        app.input = app.prompt_history[idx].clone();
                    }
                }
                return Ok(Self::mode_for_input(app));
            }
            KeyCode::Down if app.input_cursor == 0 => {
                app.prompt_history_idx = match app.prompt_history_idx {
                    None => None,
                    Some(0) => None,
                    Some(i) => Some(i - 1),
                };
                if let Some(idx) = app.prompt_history_idx {
                    app.input = app.prompt_history[idx].clone();
                } else {
                    Input::clear_input(app);
                }
                return Ok(Self::mode_for_input(app));
            }
            KeyCode::Tab => {
                let agent = app.agents.next_agent(app.snapshot.agent_name.as_str());
                let next_agent = agent.cloned();
                if let Some(agent) = next_agent {
                    if !app.harness_idle() {
                        app.status =
                            "Agent is busy; finish current turn before changing agents".to_string();
                    } else {
                        let name = agent.name.clone();
                        match actor.set_agent(agent).await {
                            Ok(snapshot) => {
                                app.save_snapshot(&snapshot);
                                app.snapshot.agent_name = name;
                            }
                            Err(err) => app.status = err,
                        }
                    }
                }
            }
            KeyCode::Enter => {
                let prompt = app.input.trim().to_string();
                if prompt.is_empty() {
                    return Ok(Self::PromptInput);
                }

                if let Some(command) = prompt.trim().strip_prefix('/') {
                    return Ok(Command::execute(app, actor, command).await);
                }

                app.queue_prompt(prompt, actor).await?;
            }
            KeyCode::Char(_) => {
                return Ok(Self::mode_for_input(app));
            }
            _ => {}
        };

        Ok(Self::PromptInput)
    }

    pub async fn handle_command(
        app: &mut TuiApp,
        key: KeyEvent,
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
                    return Command::execute(app, actor, command).await;
                }
            }
            KeyCode::Backspace | KeyCode::Delete | KeyCode::Char(_) => {
                picker.replace_filter(Command::filter_commands(&app.input, &picker.rows));
                if picker.is_empty() {
                    return InputMode::PromptInput;
                }
            }
            _ => {}
        }

        InputMode::Command { picker }
    }

    pub fn command_mode_for_input(app: &TuiApp) -> Option<Self> {
        let input = app.input.trim();

        if let Some(query) = Self::get_file_query(&app.input, app.input_cursor) {
            let files = Self::list_files();
            let filtered = Self::filter_files(&query, &files);

            return Some(Self::Files {
                picker: PickList::new(files, filtered),
            });
        }

        if input.starts_with('/') {
            let commands = Command::list();
            let filtered = Command::filter_commands(input, &commands);

            if !filtered.is_empty() {
                return Some(Self::Command {
                    picker: PickList::new(commands, filtered),
                });
            }
        }

        None
    }

    pub fn mode_for_input(app: &TuiApp) -> Self {
        Self::command_mode_for_input(app).unwrap_or(Self::PromptInput)
    }

    pub fn input_changed_key(key: KeyEvent) -> bool {
        matches!(
            key.code,
            KeyCode::Backspace | KeyCode::Delete | KeyCode::Char('u') | KeyCode::Char('k')
        ) && key.modifiers.contains(KeyModifiers::CONTROL)
            || matches!(key.code, KeyCode::Backspace | KeyCode::Delete)
            || matches!(key.code, KeyCode::Char(_))
                && !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
    }
}

pub struct Input {}

impl Input {
    pub fn handle_input_cursor(app: &mut TuiApp, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                Self::clear_input(app);
            }
            KeyCode::Backspace => {
                Self::delete_input_char_before_cursor(app);
            }
            KeyCode::Delete => {
                Self::delete_input_char_at_cursor(app);
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Self::delete_input_to_start(app);
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Self::delete_input_to_end(app);
            }
            KeyCode::Left => {
                Self::move_input_cursor_left(app);
            }
            KeyCode::Right => {
                Self::move_input_cursor_right(app);
            }
            KeyCode::Home => {
                Self::move_input_cursor_home(app);
            }
            KeyCode::End => {
                Self::move_input_cursor_end(app);
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                Self::insert_input_char(app, ch);
            }
            _ => {}
        }
    }

    fn input_len(app: &TuiApp) -> usize {
        app.input.chars().count()
    }

    fn input_cursor_byte_index(app: &TuiApp) -> usize {
        if app.input_cursor == Self::input_len(app) {
            return app.input.len();
        }

        app.input
            .char_indices()
            .nth(app.input_cursor)
            .map(|(index, _)| index)
            .unwrap_or(app.input.len())
    }

    pub fn clear_input(app: &mut TuiApp) {
        app.input.clear();
        app.input_cursor = 0;
    }

    pub fn insert_input_char(app: &mut TuiApp, ch: char) {
        let index = Self::input_cursor_byte_index(app);
        app.input.insert(index, ch);
        app.input_cursor += 1;
    }

    pub fn delete_input_char_before_cursor(app: &mut TuiApp) {
        if app.input_cursor == 0 {
            return;
        }

        app.input_cursor -= 1;
        let index = Self::input_cursor_byte_index(app);
        app.input.remove(index);
    }

    pub fn delete_input_char_at_cursor(app: &mut TuiApp) {
        if app.input_cursor == Self::input_len(app) {
            return;
        }

        let index = Self::input_cursor_byte_index(app);
        app.input.remove(index);
    }

    pub fn delete_input_to_start(app: &mut TuiApp) {
        let index = Self::input_cursor_byte_index(app);
        app.input.drain(..index);
        app.input_cursor = 0;
    }

    pub fn delete_input_to_end(app: &mut TuiApp) {
        let index = Self::input_cursor_byte_index(app);
        app.input.truncate(index);
    }

    pub fn move_input_cursor_left(app: &mut TuiApp) {
        app.input_cursor = app.input_cursor.saturating_sub(1);
    }

    pub fn move_input_cursor_right(app: &mut TuiApp) {
        app.input_cursor = (app.input_cursor + 1).min(Self::input_len(app));
    }

    pub fn move_input_cursor_home(app: &mut TuiApp) {
        app.input_cursor = 0;
    }

    pub fn move_input_cursor_end(app: &mut TuiApp) {
        app.input_cursor = Self::input_len(app);
    }

    pub fn scroll_history_bottom(app: &mut TuiApp) {
        app.history_scroll = 0;
        app.new_lines_below = 0;
    }

    pub fn scroll_history_up(app: &mut TuiApp, amount: u16) {
        app.history_scroll = app.history_scroll.saturating_add(amount);
    }

    pub fn scroll_history_down(app: &mut TuiApp, amount: u16) {
        app.history_scroll = app.history_scroll.saturating_sub(amount);
        let dismissed = amount.min(app.new_lines_below);
        app.new_lines_below = app.new_lines_below.saturating_sub(dismissed);
    }

    pub fn history_page_scroll_amount(app: &TuiApp) -> u16 {
        app.history_page_size.saturating_sub(1).max(1)
    }
}
