/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    actor::HarnessActorClient,
    agents::AgentDefinition,
    input::{InputMode, PromptEditor, picklist::PickList},
    skills::Skill,
    tui::TuiApp,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl InputMode {
    pub async fn handle_agents(
        app: &mut TuiApp,
        key: KeyEvent,
        mut picker: PickList<AgentDefinition>,
        actor: &HarnessActorClient,
    ) -> Self {
        app.editor.handle_input_cursor(key);
        match key.code {
            KeyCode::Esc => {
                app.editor.clear_input();
                return Self::PromptInput;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Self::Exit;
            }
            KeyCode::Up => {
                picker.move_up();
            }
            KeyCode::Down => {
                picker.move_down();
            }
            KeyCode::Enter => {
                let Some(selected_agent) = picker.selected_row().cloned() else {
                    app.editor.set_status("No matching agents".to_string());
                    return Self::Agents { picker };
                };
                if let Err(err) = actor.set_agent(selected_agent).await {
                    app.editor.set_status(err);
                }
            }
            _ if PromptEditor::input_changed_key(key) => {
                picker.filter(&app.editor.input);
            }
            _ => {}
        }

        Self::Agents { picker }
    }

    pub async fn handle_skills(
        app: &mut TuiApp,
        key: KeyEvent,
        mut picker: PickList<Skill>,
        actor: &HarnessActorClient,
    ) -> Self {
        app.editor.handle_input_cursor(key);
        match key.code {
            KeyCode::Esc => {
                app.editor.clear_input();
                return Self::PromptInput;
            }
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
                if let Some(skill) = picker.selected_row() {
                    match actor.run_skill(skill.name.clone()).await {
                        Ok(()) => {
                            app.editor.set_status(format!("Ran skill: {}", skill.name));
                        }
                        Err(err) => {
                            app.editor
                                .set_status(format!("Error running skill: {}", err));
                        }
                    }
                    app.editor.clear_input();
                    return Self::PromptInput;
                }
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Err(err) = actor.reload_skills().await {
                    app.editor
                        .set_status(format!("Error reloading skills: {}", err));
                }
            }
            _ if PromptEditor::input_changed_key(key) => {
                picker.filter(&app.editor.input);
            }
            _ => {}
        }
        Self::Skills { picker }
    }
}
