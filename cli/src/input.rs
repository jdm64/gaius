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
    commands::{Command, Commands},
    config::Config,
    harness::Harness,
    models::AvailableModel,
    tui::{TerminalGuard, TuiApp},
};
use crossterm::event::{self, KeyCode, KeyModifiers};
use std::error::Error;

const MAX_HISTORY: usize = 16;

pub enum InputMode {
    Exit,
    PromptInput,
    Command {
        selected: usize,
        filtered: Vec<Command>,
    },
    Session {
        selected: usize,
        sessions: Vec<String>,
    },
    Models {
        selected: usize,
        models: Vec<AvailableModel>,
    },
    Agents {
        selected: usize,
        agents: Vec<AgentDefinition>,
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
        key: event::KeyEvent,
        guard: &mut TerminalGuard,
        harness: &mut Harness,
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
            KeyCode::Enter => {
                let prompt = app.input.trim().to_string();
                if prompt.is_empty() {
                    return Ok(InputMode::PromptInput);
                }

                if let Some(command) = prompt.trim().strip_prefix('/') {
                    return Ok(Commands::execute_command(app, harness, config, command).await);
                }

                app.send_prompt(prompt, harness, guard).await?;
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
        let input = app.input.as_str();
        let query = input.strip_prefix('/')?.to_lowercase();
        let filtered: Vec<Command> = Commands::commands()
            .into_iter()
            .filter(|cmd| cmd.name.to_lowercase().contains(&query))
            .collect();

        if filtered.is_empty() {
            None
        } else {
            Some(InputMode::Command {
                selected: 0,
                filtered,
            })
        }
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
