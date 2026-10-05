/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::AgentDefinition,
    commands::Command,
    harness_actor::HarnessActorHandle,
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
        actor: &HarnessActorHandle,
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
                    app.editor.status = "No matching agents".to_string();
                    return Self::Agents { picker };
                };
                if let Some(busy) = Command::busy_status(app, "changing agents") {
                    app.editor.status = busy;
                    return Self::Agents { picker };
                }
                match actor.set_agent(selected_agent.clone()).await {
                    Ok(snapshot) => {
                        app.save_snapshot(&snapshot);
                        app.snapshot.agent_name = selected_agent.name.clone();
                        app.editor.status_clear_input(&format!(
                            "Selected agent: {}",
                            selected_agent.name
                        ));
                        return Self::PromptInput;
                    }
                    Err(err) => app.editor.status = err,
                }
            }
            _ if PromptEditor::input_changed_key(key) => {
                picker.replace_filter(filter_agents(&app.editor.input, &picker.rows));
            }
            _ => {}
        }

        Self::Agents { picker }
    }

    pub async fn handle_skills(
        app: &mut TuiApp,
        key: KeyEvent,
        mut picker: PickList<Skill>,
        actor: &HarnessActorHandle,
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
                    if let Some(busy) = Command::busy_status(app, "running a skill") {
                        app.editor.status = busy;
                        return Self::Skills { picker };
                    }
                    match actor.run_skill(skill.name.clone()).await {
                        Ok(()) => {
                            app.editor.status = format!("Ran skill: {}", skill.name);
                        }
                        Err(err) => {
                            app.editor.status = format!("Error running skill: {}", err);
                        }
                    }
                    app.editor.clear_input();
                    return Self::PromptInput;
                }
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let skills = match actor.reload_skills().await {
                    Ok(skills) => skills,
                    Err(err) => {
                        app.editor.status = format!("Error reloading skills: {}", err);
                        Vec::new()
                    }
                };
                let filtered = filter_skills(&app.editor.input, &skills);
                picker.replace_rows(skills, filtered);
                app.editor.status = format!("Reloaded {} skills", picker.rows.len());
            }
            _ if PromptEditor::input_changed_key(key) => {
                picker.replace_filter(filter_skills(&app.editor.input, &picker.rows));
            }
            _ => {}
        }
        Self::Skills { picker }
    }
}

fn filter_agents(input: &str, agents: &[AgentDefinition]) -> Vec<usize> {
    let query = input.trim().to_lowercase();
    agents
        .iter()
        .enumerate()
        .filter_map(|(index, agent)| {
            (query.is_empty() || agent.name.to_lowercase().contains(&query)).then_some(index)
        })
        .collect()
}

fn filter_skills(input: &str, skills: &[Skill]) -> Vec<usize> {
    let query = input.trim().to_lowercase();
    skills
        .iter()
        .enumerate()
        .filter_map(|(index, skill)| {
            (query.is_empty()
                || skill.name.to_lowercase().contains(&query)
                || skill.description.to_lowercase().contains(&query))
            .then_some(index)
        })
        .collect()
}
