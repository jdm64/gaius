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

            KeyCode::Enter => {
                let prompt = app.input.trim().to_string();
                if prompt.is_empty() {
                    return Ok(InputMode::PromptInput);
                }

                if let Some(command) = prompt.strip_prefix('/') {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Input;

    #[test]
    fn edits_input_at_cursor() {
        let mut app = TuiApp::new();
        Input::insert_input_char(&mut app, 'a');
        Input::insert_input_char(&mut app, 'c');

        Input::move_input_cursor_left(&mut app);
        Input::insert_input_char(&mut app, 'b');

        assert_eq!(app.input, "abc");
        assert_eq!(app.input_cursor, 2);

        Input::delete_input_char_before_cursor(&mut app);

        assert_eq!(app.input, "ac");
        assert_eq!(app.input_cursor, 1);

        Input::delete_input_char_at_cursor(&mut app);

        assert_eq!(app.input, "a");
        assert_eq!(app.input_cursor, 1);
    }

    #[test]
    fn moves_input_cursor_home_and_end() {
        let mut app = TuiApp::new();
        for ch in "prompt".chars() {
            Input::insert_input_char(&mut app, ch);
        }

        Input::move_input_cursor_home(&mut app);
        assert_eq!(app.input_cursor, 0);

        Input::move_input_cursor_end(&mut app);
        assert_eq!(app.input_cursor, 6);
    }

    #[test]
    fn edits_multibyte_input_at_cursor() {
        let mut app = TuiApp::new();
        for ch in "aéc".chars() {
            Input::insert_input_char(&mut app, ch);
        }

        Input::move_input_cursor_left(&mut app);
        Input::insert_input_char(&mut app, 'b');
        Input::move_input_cursor_left(&mut app);
        Input::delete_input_char_before_cursor(&mut app);

        assert_eq!(app.input, "abc");
        assert_eq!(app.input_cursor, 1);
    }

    #[test]
    fn deletes_input_to_start_and_end() {
        let mut app = TuiApp::new();
        for ch in "abcdef".chars() {
            Input::insert_input_char(&mut app, ch);
        }

        Input::move_input_cursor_left(&mut app);
        Input::move_input_cursor_left(&mut app);
        Input::delete_input_to_start(&mut app);

        assert_eq!(app.input, "ef");
        assert_eq!(app.input_cursor, 0);

        Input::move_input_cursor_end(&mut app);
        Input::move_input_cursor_left(&mut app);
        Input::delete_input_to_end(&mut app);

        assert_eq!(app.input, "e");
        assert_eq!(app.input_cursor, 1);
    }

    #[test]
    fn deletes_multibyte_input_to_start_and_end() {
        let mut app = TuiApp::new();
        for ch in "aé文z".chars() {
            Input::insert_input_char(&mut app, ch);
        }

        Input::move_input_cursor_left(&mut app);
        Input::move_input_cursor_left(&mut app);
        Input::delete_input_to_start(&mut app);

        assert_eq!(app.input, "文z");
        assert_eq!(app.input_cursor, 0);

        Input::move_input_cursor_right(&mut app);
        Input::delete_input_to_end(&mut app);

        assert_eq!(app.input, "文");
        assert_eq!(app.input_cursor, 1);
    }

    #[test]
    fn scrolls_history_with_saturating_offsets() {
        let mut app = TuiApp::new();

        assert_eq!(app.history_scroll, 0);

        Input::scroll_history_up(&mut app, 5);
        assert_eq!(app.history_scroll, 5);

        Input::scroll_history_down(&mut app, 2);
        assert_eq!(app.history_scroll, 3);

        Input::scroll_history_down(&mut app, 10);
        assert_eq!(app.history_scroll, 0);

        Input::scroll_history_up(&mut app, 4);
        Input::reset_history_scroll(&mut app);
        assert_eq!(app.history_scroll, 0);
    }
}
