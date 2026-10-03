/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    commands::Command,
    harness_actor::HarnessActorHandle,
    input::{InputMode, PromptEditor, picklist::PickList},
    session::Session,
    token_usage::SessionInfo,
    tui::TuiApp,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl InputMode {
    pub async fn handle_session(
        app: &mut TuiApp,
        key: KeyEvent,
        mut picker: PickList<Session>,
        actor: &HarnessActorHandle,
    ) -> Self {
        match key.code {
            KeyCode::Esc => return Self::PromptInput,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Self::Exit;
            }
            KeyCode::Up if !picker.is_empty() => {
                picker.move_up();
            }
            KeyCode::Down if !picker.is_empty() => {
                picker.move_down();
            }
            KeyCode::Enter if !picker.is_empty() => {
                let Some(session) = picker.selected_row() else {
                    return Self::Session { picker };
                };
                if let Some(session_id) = &session.id {
                    if let Some(busy) = Command::busy_status(app, "loading a session") {
                        app.editor.status = busy;
                    } else {
                        match actor.load_session(session_id.clone()).await {
                            Ok(snapshot) => {
                                app.save_snapshot(&snapshot);
                                app.clear_messages();
                                app.scroll_history_bottom();
                                app.editor.status =
                                    format!("Loaded session: {}", session.display_name());
                                app.context_tokens = None;
                                match actor.replay_history().await {
                                    Ok(snapshot) => app.save_snapshot(&snapshot),
                                    Err(err) => app.editor.status = err,
                                }
                                return Self::PromptInput;
                            }
                            Err(e) => {
                                app.editor.status = format!("Error loading session: {}", e);
                            }
                        }
                    }
                } else {
                    app.editor.status = "Error loading session: missing session id".to_string();
                }
            }
            KeyCode::Char('d')
                if key.modifiers.contains(KeyModifiers::CONTROL) && !picker.is_empty() =>
            {
                let Some(session) = picker.selected_row() else {
                    return Self::Session { picker };
                };
                if let Some(session_id) = &session.id {
                    let display_name = session.display_name();
                    if let Err(e) = Session::delete(session_id) {
                        app.editor.status = format!("Error deleting session: {}", e);
                    } else {
                        let sessions = Session::list();
                        let filtered = (0..sessions.len()).collect();
                        picker.replace_rows(sessions, filtered);
                        app.editor.status = format!("Deleted session: {}", display_name);
                    }
                } else {
                    app.editor.status = "Error deleting session: missing session id".to_string();
                }
            }
            KeyCode::Char('e')
                if key.modifiers.contains(KeyModifiers::CONTROL) && !picker.is_empty() =>
            {
                let Some(session) = picker.selected_row() else {
                    return Self::Session { picker };
                };
                app.editor.input = session.display_name();
                app.editor.cursor = app.editor.input.chars().count();
                app.editor.status = "Rename session".to_string();
                return Self::SessionRename { picker };
            }
            KeyCode::Char('o')
                if key.modifiers.contains(KeyModifiers::CONTROL) && !picker.is_empty() =>
            {
                let Some(session) = picker.selected_row() else {
                    return Self::Session { picker };
                };
                match session.export() {
                    Ok(path) => {
                        app.editor.status = format!("Exported session to {}", path);
                    }
                    Err(e) => {
                        app.editor.status = format!("Error exporting session: {}", e);
                    }
                }
            }
            _ => {}
        };

        Self::Session { picker }
    }
}

impl PromptEditor {
    pub fn handle_session_rename(
        &mut self,
        key: KeyEvent,
        mut picker: PickList<Session>,
    ) -> InputMode {
        match key.code {
            KeyCode::Esc => {
                self.clear_input();
                return InputMode::Session { picker };
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return InputMode::Exit;
            }
            KeyCode::Enter => {
                let new_name = self.input.trim().to_string();
                if new_name.is_empty() {
                    self.status = "Session name cannot be empty".to_string();
                    return InputMode::SessionRename { picker };
                }

                if picker.is_empty() {
                    self.status_clear_input("No session selected");
                    return InputMode::Session { picker };
                }

                let selected_id = picker.selected_row().and_then(|session| session.id.clone());
                let Some(session) = picker.selected_row_mut() else {
                    self.status_clear_input("No session selected");
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
                        self.status_clear_input(&format!("Renamed session: {}", new_name));
                        return InputMode::Session { picker };
                    }
                    Err(e) => {
                        self.status = format!("Error renaming session: {}", e);
                        return InputMode::SessionRename { picker };
                    }
                }
            }
            _ => {
                self.handle_input_cursor(key);
            }
        }

        InputMode::SessionRename { picker }
    }

    pub fn handle_session_info(key: KeyEvent, info: SessionInfo) -> InputMode {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => InputMode::PromptInput,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => InputMode::Exit,
            _ => InputMode::SessionInfo { info },
        }
    }
}
