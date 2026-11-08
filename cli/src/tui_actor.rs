/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    actor::{CommandReply, HarnessCommand, OperationResult},
    agents::Agents,
    harness::HarnessSnapshot,
    input::{InputMode, picklist::PickList},
    models::{ModelDef, RecentModelDef},
    skills::Skill,
    token_usage::SessionInfo,
    tui::{TuiApp, TuiMessage},
};

impl TuiApp {
    pub fn handle_command_reply(&mut self, reply: CommandReply) -> Option<HarnessCommand> {
        match reply {
            CommandReply::ModelSet(result) => self.handle_model_set(result),
            CommandReply::AgentSet(result) => self.handle_agent_set(result),
            CommandReply::AgentRebuilt(result) => self.handle_agent_rebuilt(result),
            CommandReply::SessionCreated(result) => self.handle_session_created(result),
            CommandReply::SessionForked(result) => self.handle_session_forked(result),
            CommandReply::SessionLoaded(result) => return self.handle_session_loaded(result),
            CommandReply::StreamingToggled(result) => self.handle_streaming_toggled(result),
            CommandReply::PlanModeToggled(result) => self.handle_plan_mode_toggled(result),
            CommandReply::SkillsListed(result) => self.handle_skills_listed(result),
            CommandReply::SkillsReloaded(result) => self.handle_skills_reloaded(result),
            CommandReply::SessionInfo(result) => self.handle_session_info(result),
        }
        None
    }

    pub fn handle_model_set(&mut self, result: OperationResult<ModelDef>) {
        match result {
            Ok(model) => {
                self.snapshot.model = model.clone();
                let _ = RecentModelDef::add(&model);
                if self.waiting_for_answer() {
                    return;
                }
                self.editor
                    .status_clear_input(format!("Selected model: {}", model.label()));
                self.set_mode(InputMode::PromptInput);
            }
            Err(err) => self.editor.set_status(err),
        }
    }

    pub fn handle_agent_set(&mut self, result: OperationResult<String>) {
        match result {
            Ok(name) => {
                self.snapshot.agent_name = name.clone();
                if let Err(err) = Agents::save_last_name(&name) {
                    self.editor
                        .set_status(format!("Failed to save last agent: {}", err));
                }
                if self.waiting_for_answer() {
                    return;
                }
                let status = format!("Selected agent: {}", name);
                self.editor.status_clear_input(status);
                self.set_mode(InputMode::PromptInput);
            }
            Err(err) => self.editor.set_status(err),
        }
    }

    pub fn handle_agent_rebuilt(&mut self, result: OperationResult<String>) {
        match result {
            Ok(name) => {
                self.snapshot.agent_name = name;
                self.editor.set_status("Rebuilt agent and system prompt");
            }
            Err(err) => self.editor.set_status(err),
        }
    }

    pub fn handle_session_created(&mut self, result: OperationResult<HarnessSnapshot>) {
        match result {
            Ok(snapshot) => {
                self.save_snapshot(&snapshot);
                self.clear_messages();
                self.history.scroll_bottom();
                self.editor.set_status("New session created");
                self.context_tokens = None;
            }
            Err(err) => self.editor.set_status(err),
        }
    }

    pub fn handle_session_forked(&mut self, result: OperationResult<HarnessSnapshot>) {
        match result {
            Ok(snapshot) => {
                self.save_snapshot(&snapshot);
                self.push_message(TuiMessage::SystemMessage(format!(
                    "Session forked: {}",
                    snapshot
                        .session_id
                        .clone()
                        .unwrap_or("<unknown>".to_string()),
                )));
                self.editor.set_status("Forked session");
                self.history.scroll_bottom();
            }
            Err(err) => self.editor.set_status(err),
        }
    }

    pub fn handle_session_loaded(
        &mut self,
        result: OperationResult<HarnessSnapshot>,
    ) -> Option<HarnessCommand> {
        match result {
            Ok(snapshot) => {
                self.save_snapshot(&snapshot);
                self.clear_messages();
                self.history.scroll_bottom();
                let name = snapshot
                    .session_id
                    .clone()
                    .unwrap_or_else(|| "<unknown>".to_string());
                self.editor.set_status(format!("Loaded session: {}", name));
                self.set_mode(InputMode::PromptInput);
                self.context_tokens = None;
                Some(HarnessCommand::ReplayHistory)
            }
            Err(err) => {
                self.editor
                    .set_status(format!("Error loading session: {}", err));
                None
            }
        }
    }

    pub fn handle_streaming_toggled(&mut self, result: OperationResult<bool>) {
        match result {
            Ok(streaming) => {
                self.snapshot.streaming = streaming;
                self.editor.set_status(format!("Streaming = {}", streaming));
            }
            Err(err) => self.editor.set_status(err),
        }
    }

    pub fn handle_plan_mode_toggled(&mut self, result: OperationResult<bool>) {
        match result {
            Ok(plan_mode) => {
                self.snapshot.plan_mode_on = plan_mode;
                self.editor.set_status(format!("Plan mode = {}", plan_mode));
            }
            Err(err) => self.editor.set_status(err),
        }
    }

    pub fn handle_skills_listed(&mut self, result: OperationResult<Vec<Skill>>) {
        match result {
            Ok(skills) => {
                if self.waiting_for_answer() {
                    return;
                }
                self.editor.clear_input();
                self.editor.set_status("");
                self.editor.mode = InputMode::Skills {
                    picker: PickList::all(skills),
                };
            }
            Err(err) => self
                .editor
                .set_status(format!("Error loading skills: {}", err)),
        }
    }

    pub fn handle_skills_reloaded(&mut self, result: OperationResult<Vec<Skill>>) {
        match result {
            Ok(skills) => {
                let count = skills.len();
                if let InputMode::Skills { picker } = &mut self.editor.mode {
                    picker.replace_rows(skills);
                    picker.filter(&self.editor.input);
                }
                self.editor.set_status(format!("Reloaded {} skills", count));
            }
            Err(err) => self
                .editor
                .set_status(format!("Error reloading skills: {}", err)),
        }
    }

    pub fn handle_session_info(&mut self, result: OperationResult<SessionInfo>) {
        match result {
            Ok(info) => {
                if self.waiting_for_answer() {
                    return;
                }
                self.editor.clear_input();
                self.editor.mode = InputMode::SessionInfo { info };
            }
            Err(err) => self
                .editor
                .set_status(format!("Error getting session info: {}", err)),
        }
    }
}
