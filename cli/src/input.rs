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
    dirs::Dirs,
    harness_actor::HarnessActorHandle,
    input::{file::FileEntry, model::ProviderInfoRow, picklist::PickList},
    models::{ModelPickerRow, ReasoningEffort},
    session::Session,
    skills::Skill,
    token_usage::SessionInfo,
    tui::TuiApp,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{error::Error, fs};
use tokio::sync::oneshot::Sender;

const MAX_HISTORY: usize = 16;

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
        let mode = std::mem::replace(&mut app.editor.mode, Self::PromptInput);
        app.editor.mode = match mode {
            Self::PromptInput => Self::handle_prompt_input(app, key, actor).await?,
            Self::Command { picker } => Self::handle_command(app, key, picker, actor).await,
            Self::Session { picker } => Self::handle_session(app, key, picker, actor).await,
            Self::SessionRename { picker } => app.editor.handle_session_rename(key, picker),
            Self::Models { picker } => Self::handle_models(app, key, picker, actor).await,
            Self::AddProvider { picker } => Self::handle_provider_add(app, key, picker).await,
            Self::Agents { picker } => Self::handle_agents(app, key, picker, actor).await,
            Self::Files { picker } => app.editor.handle_files(key, picker).await,
            Self::Skills { picker } => Self::handle_skills(app, key, picker, actor).await,
            Self::Reasoning { picker } => Self::handle_reasoning(app, key, picker, actor).await,
            Self::Question {
                title: _,
                options: _,
                selected: _,
            } => Self::PromptInput,
            Self::SessionInfo { info } => PromptEditor::handle_session_info(key, info),
            Self::Exit => Self::Exit,
        };
        Ok(())
    }

    pub async fn handle_prompt_input(
        app: &mut TuiApp,
        key: KeyEvent,
        actor: &HarnessActorHandle,
    ) -> Result<Self, Box<dyn Error>> {
        app.editor.handle_input_cursor(key);
        match key.code {
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.editor.status = "Cancelling agent...".to_string();
                actor.cancel().await?;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(Self::Exit);
            }
            KeyCode::Backspace | KeyCode::Delete => {
                return Ok(app.editor.mode_for_input());
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(app.editor.mode_for_input());
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(app.editor.mode_for_input());
            }
            KeyCode::Up if app.editor.cursor == 0 => {
                let len = app.editor.history.len();
                if len > 0 {
                    app.editor.history_idx = match app.editor.history_idx {
                        None => Some(0),
                        Some(i) if i + 1 < len => Some(i + 1),
                        Some(i) => Some(i),
                    };
                    if let Some(idx) = app.editor.history_idx {
                        app.editor.input = app.editor.history[idx].clone();
                    }
                }
                return Ok(app.editor.mode_for_input());
            }
            KeyCode::Down if app.editor.cursor == 0 => {
                app.editor.history_idx = match app.editor.history_idx {
                    None => None,
                    Some(0) => None,
                    Some(i) => Some(i - 1),
                };
                if let Some(idx) = app.editor.history_idx {
                    app.editor.input = app.editor.history[idx].clone();
                } else {
                    app.editor.clear_input();
                }
                return Ok(app.editor.mode_for_input());
            }
            KeyCode::Tab => {
                let agent = app.agents.next_agent(app.snapshot.agent_name.as_str());
                let next_agent = agent.cloned();
                if let Some(agent) = next_agent {
                    if !app.harness_idle() {
                        app.editor.status =
                            "Agent is busy; finish current turn before changing agents".to_string();
                    } else {
                        let name = agent.name.clone();
                        match actor.set_agent(agent).await {
                            Ok(snapshot) => {
                                app.save_snapshot(&snapshot);
                                app.snapshot.agent_name = name;
                            }
                            Err(err) => app.editor.status = err,
                        }
                    }
                }
            }
            KeyCode::Enter => {
                let prompt = app.editor.input.trim().to_string();
                if prompt.is_empty() {
                    return Ok(Self::PromptInput);
                }

                if let Some(name) = prompt.trim().strip_prefix('/') {
                    let mode = match Command::from_name(name) {
                        Some(command) => Command::execute(app, actor, command).await,
                        None => {
                            app.editor.status = format!("Unknown command: {name}");
                            InputMode::PromptInput
                        }
                    };
                    return Ok(mode);
                }

                app.queue_prompt(prompt, actor).await?;
            }
            KeyCode::Char(_) => {
                return Ok(app.editor.mode_for_input());
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
    ) -> Self {
        app.editor.handle_input_cursor(key);
        match key.code {
            KeyCode::Esc => return Self::PromptInput,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Self::Exit;
            }
            KeyCode::Up => {
                picker.move_up();
            }
            KeyCode::Down => {
                picker.move_down();
            }
            KeyCode::Enter if !picker.is_empty() => {
                if let Some(command) = picker.selected_row().copied() {
                    return Command::execute(app, actor, command).await;
                }
            }
            KeyCode::Backspace | KeyCode::Delete | KeyCode::Char(_) => {
                picker.replace_filter(Command::filter_commands(&app.editor.input, &picker.rows));
                if picker.is_empty() {
                    return Self::PromptInput;
                }
            }
            _ => {}
        }

        Self::Command { picker }
    }
}

pub struct PromptEditor {
    pub mode: InputMode,
    pub input: String,
    pub cursor: usize,
    pub status: String,
    pub history: Vec<String>,
    pub history_idx: Option<usize>,
    pub answer_tx: Option<Sender<String>>,
}

impl PromptEditor {
    pub fn new() -> Self {
        Self {
            mode: InputMode::PromptInput,
            input: String::new(),
            cursor: 0,
            status: String::new(),
            history: Vec::new(),
            history_idx: None,
            answer_tx: None,
        }
    }

    pub fn command_mode_for_input(&self) -> Option<InputMode> {
        let input = self.input.trim();

        if let Some(query) = Self::get_file_query(&self.input, self.cursor) {
            let files = Self::list_files();
            let filtered = Self::filter_files(&query, &files);

            return Some(InputMode::Files {
                picker: PickList::new(files, filtered),
            });
        }

        if input.starts_with('/') {
            let commands = Command::list();
            let filtered = Command::filter_commands(input, &commands);

            if !filtered.is_empty() {
                return Some(InputMode::Command {
                    picker: PickList::new(commands, filtered),
                });
            }
        }

        None
    }

    pub fn mode_for_input(&self) -> InputMode {
        self.command_mode_for_input()
            .unwrap_or(InputMode::PromptInput)
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

    pub fn handle_input_cursor(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.clear_input();
            }
            KeyCode::Backspace => {
                self.delete_input_char_before_cursor();
            }
            KeyCode::Delete => {
                self.delete_input_char_at_cursor();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.delete_input_to_start();
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.delete_input_to_end();
            }
            KeyCode::Left => {
                self.move_input_cursor_left();
            }
            KeyCode::Right => {
                self.move_input_cursor_right();
            }
            KeyCode::Home => {
                self.move_input_cursor_home();
            }
            KeyCode::End => {
                self.move_input_cursor_end();
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.insert_input_char(ch);
            }
            _ => {}
        }
    }

    fn input_len(&self) -> usize {
        self.input.chars().count()
    }

    fn input_cursor_byte_index(&self) -> usize {
        if self.cursor == self.input_len() {
            return self.input.len();
        }

        self.input
            .char_indices()
            .nth(self.cursor)
            .map(|(index, _)| index)
            .unwrap_or(self.input.len())
    }

    pub fn status_clear_input(&mut self, msg: &str) {
        self.status = msg.to_string();
        self.clear_input();
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
        self.cursor = 0;
    }

    pub fn insert_input_char(&mut self, ch: char) {
        let index = self.input_cursor_byte_index();
        self.input.insert(index, ch);
        self.cursor += 1;
    }

    pub fn delete_input_char_before_cursor(&mut self) {
        if self.cursor == 0 {
            return;
        }

        self.cursor -= 1;
        let index = self.input_cursor_byte_index();
        self.input.remove(index);
    }

    pub fn delete_input_char_at_cursor(&mut self) {
        if self.cursor == self.input_len() {
            return;
        }

        let index = self.input_cursor_byte_index();
        self.input.remove(index);
    }

    pub fn delete_input_to_start(&mut self) {
        let index = self.input_cursor_byte_index();
        self.input.drain(..index);
        self.cursor = 0;
    }

    pub fn delete_input_to_end(&mut self) {
        let index = self.input_cursor_byte_index();
        self.input.truncate(index);
    }

    pub fn move_input_cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_input_cursor_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.input_len());
    }

    pub fn move_input_cursor_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_input_cursor_end(&mut self) {
        self.cursor = self.input_len();
    }

    pub fn update_prompt_history(&mut self, prompt: String) {
        if prompt.is_empty() {
            return;
        }

        if let Some(idx) = self.history_idx
            && idx < self.history.len()
        {
            self.history[idx] = prompt.clone();
            if idx != 0 {
                self.history.swap(0, idx);
            }
        } else {
            self.history.insert(0, prompt.clone());
        }

        if self.history.len() > MAX_HISTORY {
            self.history.truncate(MAX_HISTORY);
        }

        self.history_idx = None;

        if let Err(e) = self.save_prompt_history() {
            eprintln!("Failed to save prompt history: {}", e);
        }
    }

    pub fn load_prompt_history(&mut self) -> Result<(), Box<dyn Error>> {
        let path = Dirs::prompt_history_file()?;
        if path.exists() {
            let contents = fs::read_to_string(&path)?;
            self.history = serde_json::from_str(&contents).unwrap_or_default();
        }
        self.history_idx = None;
        Ok(())
    }

    pub fn save_prompt_history(&self) -> Result<(), Box<dyn Error>> {
        let path = Dirs::prompt_history_file()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(&self.history)?;
        fs::write(path, contents)?;
        Ok(())
    }

    pub fn handle_question_key(&mut self, key: KeyEvent) {
        let mode = std::mem::replace(&mut self.mode, InputMode::PromptInput);
        let InputMode::Question {
            title,
            options,
            mut selected,
        } = mode
        else {
            self.mode = mode;
            return;
        };

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.answer_question(String::new());
                self.mode = InputMode::Exit;
            }
            KeyCode::Esc | KeyCode::Tab => {
                self.answer_question(String::new());
                self.clear_input();
                self.mode = InputMode::PromptInput;
            }
            KeyCode::Enter => {
                let answer = options.get(selected).cloned().unwrap_or_default();
                let details = self.input.trim().to_string();
                let response = [answer, details]
                    .iter()
                    .filter(|part| !part.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
                self.answer_question(response);
                self.clear_input();
                self.mode = InputMode::PromptInput;
            }
            KeyCode::Up => {
                selected = selected.saturating_sub(1);
                self.mode = InputMode::Question {
                    title,
                    options,
                    selected,
                };
            }
            KeyCode::Down => {
                if selected + 1 < options.len() {
                    selected += 1;
                }
                self.mode = InputMode::Question {
                    title,
                    options,
                    selected,
                };
            }
            _ => {
                self.handle_input_cursor(key);
                self.mode = InputMode::Question {
                    title,
                    options,
                    selected,
                };
            }
        }
    }

    fn answer_question(&mut self, answer: String) {
        if let Some(answer_tx) = self.answer_tx.take() {
            let _ = answer_tx.send(answer);
        }
    }
}

impl Default for PromptEditor {
    fn default() -> Self {
        Self::new()
    }
}
