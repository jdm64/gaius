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
    commands::{Command, Commands, input_changed_key},
    config::Config,
    harness_actor::HarnessActorHandle,
    models::ModelPickerRow,
    session::Session,
    tui::TuiApp,
};
use crossterm::event::{self, KeyCode, KeyEvent, KeyModifiers};
use std::{error::Error, path::PathBuf};

const MAX_HISTORY: usize = 16;

fn char_pos_to_byte_index(input: &str, char_pos: usize) -> usize {
    input
        .char_indices()
        .nth(char_pos)
        .map(|(index, _)| index)
        .unwrap_or(input.len())
}

pub struct PickList<T> {
    pub selected: usize,
    pub rows: Vec<T>,
    pub filtered: Vec<usize>,
}

impl<T> PickList<T> {
    pub fn new(rows: Vec<T>, filtered: Vec<usize>) -> Self {
        let mut list = Self {
            selected: 0,
            rows,
            filtered,
        };
        list.clamp_selected();
        list
    }

    pub fn all(rows: Vec<T>) -> Self {
        let filtered = (0..rows.len()).collect();
        Self::new(rows, filtered)
    }

    pub fn is_empty(&self) -> bool {
        self.filtered.is_empty()
    }

    pub fn selected_row_index(&self) -> Option<usize> {
        self.filtered.get(self.selected).copied()
    }

    pub fn selected_row(&self) -> Option<&T> {
        self.selected_row_index()
            .and_then(|index| self.rows.get(index))
    }

    pub fn selected_row_mut(&mut self) -> Option<&mut T> {
        let index = self.selected_row_index()?;
        self.rows.get_mut(index)
    }

    pub fn move_up(&mut self) {
        self.selected = wrap_selection(self.selected as i32 - 1, self.filtered.len());
    }

    pub fn move_down(&mut self) {
        self.selected = wrap_selection(self.selected as i32 + 1, self.filtered.len());
    }

    pub fn replace_filter(&mut self, filtered: Vec<usize>) {
        self.filtered = filtered;
        self.clamp_selected();
    }

    pub fn replace_rows(&mut self, rows: Vec<T>, filtered: Vec<usize>) {
        self.rows = rows;
        self.filtered = filtered;
        self.clamp_selected();
    }

    pub fn clamp_selected(&mut self) {
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
    }

    pub fn visible_row_range(&self, max_visible: usize) -> (usize, usize) {
        let visible = self.rows.len().clamp(1, max_visible);
        let selected_row = self.selected_row_index().unwrap_or(0);
        let start = selected_row.saturating_add(1).saturating_sub(visible);
        let end = (start + visible).min(self.rows.len());
        (start, end)
    }
}

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
    Agents {
        picker: PickList<AgentDefinition>,
    },
    Files {
        picker: PickList<FileEntry>,
    },
    Question {
        title: String,
        options: Vec<String>,
        selected: usize,
    },
}

pub struct Input {}

impl Input {
    pub fn handle_input_cursor(app: &mut TuiApp, key: event::KeyEvent) {
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

    pub async fn handle_prompt_input(
        app: &mut TuiApp,
        key: KeyEvent,
        actor: &HarnessActorHandle,
        config: &Config,
    ) -> Result<InputMode, Box<dyn Error>> {
        Self::handle_input_cursor(app, key);
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(InputMode::Exit);
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
                    Self::clear_input(app);
                }
                return Ok(Self::mode_for_input(app));
            }
            KeyCode::Tab => {
                let agent = app.agents.next_agent(app.agent_name.as_str());
                let next_agent = agent.cloned();
                if let Some(agent) = next_agent {
                    if !app.harness_idle() {
                        app.status =
                            "Agent is busy; finish current turn before changing agents".to_string();
                    } else {
                        let name = agent.name.clone();
                        match actor.set_agent(agent).await {
                            Ok(snapshot) => {
                                app.apply_snapshot(&snapshot);
                                app.agent_name = name;
                            }
                            Err(err) => app.status = err,
                        }
                    }
                }
            }
            KeyCode::Enter => {
                let prompt = app.input.trim().to_string();
                if prompt.is_empty() {
                    return Ok(InputMode::PromptInput);
                }

                if let Some(command) = prompt.trim().strip_prefix('/') {
                    return Ok(Commands::execute_command(app, actor, config, command).await);
                }

                app.queue_prompt(prompt, actor).await?;
            }
            KeyCode::Char(_) => {
                return Ok(Self::mode_for_input(app));
            }
            _ => {}
        };

        Ok(InputMode::PromptInput)
    }

    pub fn update_prompt_history(app: &mut TuiApp, prompt: String) {
        if prompt.is_empty() {
            return;
        }

        if let Some(idx) = app.prompt_history_idx
            && idx < app.prompt_history.len()
        {
            app.prompt_history[idx] = prompt.clone();
            if idx != 0 {
                app.prompt_history.swap(0, idx);
            }
        } else {
            app.prompt_history.insert(0, prompt.clone());
        }

        if app.prompt_history.len() > MAX_HISTORY {
            app.prompt_history.truncate(MAX_HISTORY);
        }

        app.prompt_history_idx = None;

        if let Err(e) = app.save_prompt_history() {
            eprintln!("Failed to save prompt history: {}", e);
        }
    }

    fn command_mode_for_input(app: &TuiApp) -> Option<InputMode> {
        let input = app.input.trim();

        if let Some(query) = Input::get_file_query(&app.input, app.input_cursor) {
            let files = list_files();
            let filtered = Input::filter_files(&query, &files);

            return Some(InputMode::Files {
                picker: PickList::new(files, filtered),
            });
        }

        if input.starts_with('/') {
            let commands = Commands::commands();
            let filtered = Self::filter_commands(input, &commands);

            if !filtered.is_empty() {
                return Some(InputMode::Command {
                    picker: PickList::new(commands, filtered),
                });
            }
        }

        None
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

    pub fn filter_model_rows(input: &str, rows: &[ModelPickerRow]) -> Vec<usize> {
        let query = input.trim().to_lowercase();
        rows.iter()
            .enumerate()
            .filter_map(|(index, row)| match row {
                ModelPickerRow::Model(model) | ModelPickerRow::RecentModel(model)
                    if query.is_empty() || model.id.to_lowercase().contains(&query) =>
                {
                    Some(index)
                }
                ModelPickerRow::Header(_)
                | ModelPickerRow::Separator
                | ModelPickerRow::Model(_)
                | ModelPickerRow::RecentModel(_) => None,
            })
            .collect()
    }

    pub fn filter_agents(input: &str, agents: &[AgentDefinition]) -> Vec<usize> {
        let query = input.trim().to_lowercase();
        agents
            .iter()
            .enumerate()
            .filter_map(|(index, agent)| {
                (query.is_empty() || agent.name.to_lowercase().contains(&query)).then_some(index)
            })
            .collect()
    }

    pub async fn handle_files_mode(
        app: &mut TuiApp,
        key: event::KeyEvent,
        mut picker: PickList<FileEntry>,
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
                if let Some(file) = picker.selected_row() {
                    app.input = Input::replace_file_query(&file.name, &app.input, app.input_cursor);
                    app.input_cursor = app.input.chars().count();
                }
                return InputMode::PromptInput;
            }
            _ if input_changed_key(key) => {
                if let Some(query) = Input::get_file_query(&app.input, app.input_cursor) {
                    let filtered = Input::filter_files(&query, &picker.rows);
                    picker.replace_filter(filtered);
                } else {
                    return InputMode::PromptInput;
                }
            }
            _ => {}
        }

        InputMode::Files { picker }
    }

    pub fn filter_files(input: &str, files: &[FileEntry]) -> Vec<usize> {
        let query = input
            .strip_prefix('@')
            .unwrap_or(input)
            .trim()
            .to_lowercase();
        files
            .iter()
            .enumerate()
            .filter_map(|(index, file)| {
                (query.is_empty() || file.name.to_lowercase().contains(&query)).then_some(index)
            })
            .collect()
    }

    pub fn get_file_query(input: &str, cursor_pos: usize) -> Option<String> {
        let cursor_byte = char_pos_to_byte_index(input, cursor_pos);
        let input_before_cursor = &input[..cursor_byte];
        let query_start = input_before_cursor
            .char_indices()
            .rev()
            .find_map(|(index, ch)| ch.is_whitespace().then_some(index + ch.len_utf8()))
            .unwrap_or(0);

        input_before_cursor[query_start..]
            .strip_prefix('@')
            .map(ToString::to_string)
    }

    pub fn replace_file_query(filename: &str, input: &str, cursor_pos: usize) -> String {
        let cursor_byte = char_pos_to_byte_index(input, cursor_pos);
        let token_start = input[..cursor_byte]
            .char_indices()
            .rev()
            .find_map(|(index, ch)| ch.is_whitespace().then_some(index + ch.len_utf8()))
            .unwrap_or(0);

        if !input[token_start..].starts_with('@') {
            return input.to_string();
        }

        let token_end = input[cursor_byte..]
            .char_indices()
            .find_map(|(index, ch)| ch.is_whitespace().then_some(cursor_byte + index))
            .unwrap_or(input.len());

        format!(
            "{}{}{}",
            &input[..token_start],
            filename,
            &input[token_end..]
        )
    }

    pub fn mode_for_input(app: &TuiApp) -> InputMode {
        Self::command_mode_for_input(app).unwrap_or(InputMode::PromptInput)
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

    pub fn reset_history_scroll(app: &mut TuiApp) {
        app.history_scroll = 0;
    }

    pub fn scroll_history_up(app: &mut TuiApp, amount: u16) {
        app.history_scroll = app.history_scroll.saturating_add(amount);
    }

    pub fn scroll_history_down(app: &mut TuiApp, amount: u16) {
        app.history_scroll = app.history_scroll.saturating_sub(amount);
    }

    pub fn history_page_scroll_amount(app: &TuiApp) -> u16 {
        app.history_page_size.saturating_sub(1).max(1)
    }
}

fn wrap_selection(i: i32, n: usize) -> usize {
    if n > 0 {
        let m = n as i32;
        ((i % m + m) % m) as usize
    } else {
        0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
}

pub fn list_files() -> Vec<FileEntry> {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut entries: Vec<FileEntry> = Vec::new();

    fn walk_dir(path: &PathBuf, relative_path: &str, entries: &mut Vec<FileEntry>) {
        if let Ok(read_dir) = std::fs::read_dir(path) {
            for entry in read_dir.filter_map(|e| e.ok()) {
                let entry_path = entry.path();
                let name = entry_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                // Skip ignored directories
                if [".git", "target", "node_modules", "build", "dist", "bin"]
                    .iter()
                    .any(|&ignored| name == ignored)
                {
                    continue;
                }

                let rel_name = if relative_path.is_empty() {
                    name
                } else {
                    format!("{}/{}", relative_path, name)
                };
                entries.push(FileEntry {
                    name: rel_name.clone(),
                    path: entry_path.clone(),
                });
                if entry_path.is_dir() {
                    walk_dir(&entry_path, &rel_name, entries);
                }
            }
        }
    }

    walk_dir(&current_dir, "", &mut entries);

    entries
}
