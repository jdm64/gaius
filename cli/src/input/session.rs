/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    commands::Command,
    harness_actor::HarnessActorHandle,
    input::{Input, InputMode, picklist::PickList},
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
                        app.status = busy;
                    } else {
                        match actor.load_session(session_id.clone()).await {
                            Ok(snapshot) => {
                                app.save_snapshot(&snapshot);
                                app.clear_messages();
                                Input::scroll_history_bottom(app);
                                app.status = format!("Loaded session: {}", session.display_name());
                                app.context_tokens = None;
                                match actor.replay_history().await {
                                    Ok(snapshot) => app.save_snapshot(&snapshot),
                                    Err(err) => app.status = err,
                                }
                                return Self::PromptInput;
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
                    return Self::Session { picker };
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
                    return Self::Session { picker };
                };
                app.input = session.display_name();
                app.input_cursor = app.input.chars().count();
                app.status = "Rename session".to_string();
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
                        app.status = format!("Exported session to {}", path);
                    }
                    Err(e) => {
                        app.status = format!("Error exporting session: {}", e);
                    }
                }
            }
            _ => {}
        };

        Self::Session { picker }
    }

    pub fn handle_session_rename(
        app: &mut TuiApp,
        key: KeyEvent,
        mut picker: PickList<Session>,
    ) -> Self {
        match key.code {
            KeyCode::Esc => {
                Input::clear_input(app);
                return Self::Session { picker };
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Self::Exit;
            }
            KeyCode::Enter => {
                let new_name = app.input.trim().to_string();
                if new_name.is_empty() {
                    app.status = "Session name cannot be empty".to_string();
                    return Self::SessionRename { picker };
                }

                if picker.is_empty() {
                    app.status = "No session selected".to_string();
                    Input::clear_input(app);
                    return Self::Session { picker };
                }

                let selected_id = picker.selected_row().and_then(|session| session.id.clone());
                let Some(session) = picker.selected_row_mut() else {
                    app.status = "No session selected".to_string();
                    Input::clear_input(app);
                    return Self::Session { picker };
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
                        return Self::Session { picker };
                    }
                    Err(e) => {
                        app.status = format!("Error renaming session: {}", e);
                        return Self::SessionRename { picker };
                    }
                }
            }
            _ => {
                Input::handle_input_cursor(app, key);
            }
        }

        Self::SessionRename { picker }
    }

    pub fn handle_session_info(key: KeyEvent, info: SessionInfo) -> Self {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => Self::PromptInput,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Self::Exit,
            _ => Self::SessionInfo { info },
        }
    }
}
