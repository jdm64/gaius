/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::AgentDefinition,
    harness::{Harness, HarnessMsg, HarnessSnapshot, UserRequest},
    models::ModelDef,
    skills::Skill,
    token_usage::SessionInfo,
};
use futures::executor;
use std::{
    io::{self, Error},
    path::PathBuf,
    thread,
};
use tokio::{
    runtime::Builder,
    sync::{
        mpsc::{self, Receiver, Sender, UnboundedReceiver, UnboundedSender},
        oneshot,
    },
};

pub type OperationResult<T> = Result<T, String>;
pub type AnswerSender = oneshot::Sender<String>;

const ACTOR_STOPPED: &str = "Harness actor stopped";

#[derive(Debug)]
pub enum HarnessEvent {
    Message(HarnessMsg),
    Command(CommandReply),
    AskUser {
        title: String,
        options: Vec<String>,
        answer_tx: AnswerSender,
    },
    TurnFinished(HarnessSnapshot),
    RequestFailed(String, HarnessSnapshot),
    HistoryReplayed(Vec<HarnessMsg>),
    ShutdownComplete(OperationResult<HarnessSnapshot>),
}

pub enum HarnessCommand {
    RunPrompt(String),
    RunSkill(String),
    SetModel(ModelDef),
    SetAgent(AgentDefinition),
    NewSession(Option<PathBuf>),
    ForkSession,
    LoadSession(String),
    CloseSession,
    ToggleStreaming,
    TogglePlanMode,
    ReplayHistory,
    Compact,
    Cancel,
    Info,
    GetSkills,
    ReloadSkills,
    RebuildAgent(AgentDefinition),
    Shutdown,
}

#[derive(Debug)]
pub enum CommandReply {
    ModelSet(OperationResult<ModelDef>),
    AgentSet(OperationResult<String>),
    AgentRebuilt(OperationResult<String>),
    SessionCreated(OperationResult<HarnessSnapshot>),
    SessionForked(OperationResult<HarnessSnapshot>),
    SessionLoaded(OperationResult<HarnessSnapshot>),
    SessionClosed(OperationResult<HarnessSnapshot>),
    StreamingToggled(OperationResult<bool>),
    PlanModeToggled(OperationResult<bool>),
    SkillsListed(OperationResult<Vec<Skill>>),
    SkillsReloaded(OperationResult<Vec<Skill>>),
    SessionInfo(OperationResult<SessionInfo>),
    Busy(String),
}

impl From<CommandReply> for HarnessEvent {
    fn from(reply: CommandReply) -> Self {
        HarnessEvent::Command(reply)
    }
}

pub struct HarnessActorClient {
    tx: Sender<HarnessCommand>,
    pub rx: UnboundedReceiver<HarnessEvent>,
}

impl HarnessActorClient {
    pub fn new(harness: Harness) -> io::Result<HarnessActorClient> {
        let (tx, command_rx) = mpsc::channel(64);
        let (event_tx, rx) = mpsc::unbounded_channel();

        // run actor in new thread so it doesn't block UI when tools are called.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        thread::Builder::new()
            .name("gaius-harness-actor".to_string())
            .spawn(move || {
                let rt = match Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                let mut server = HarnessActorServer::new(harness, command_rx, event_tx);
                rt.block_on(server.run_actor());
            })?;

        ready_rx
            .recv()
            .map_err(|_| Error::other("harness actor thread panicked during startup"))??;

        Ok(HarnessActorClient { tx, rx })
    }

    pub async fn send(&self, command: HarnessCommand) -> Result<(), String> {
        self.tx
            .send(command)
            .await
            .map_err(|_| ACTOR_STOPPED.to_string())
    }

    pub async fn run_prompt(&self, prompt: String) -> Result<(), String> {
        self.send(HarnessCommand::RunPrompt(prompt)).await
    }

    pub async fn run_skill(&self, name: String) -> Result<(), String> {
        self.send(HarnessCommand::RunSkill(name)).await
    }

    pub async fn get_skills(&self) -> Result<(), String> {
        self.send(HarnessCommand::GetSkills).await
    }

    pub async fn reload_skills(&self) -> Result<(), String> {
        self.send(HarnessCommand::ReloadSkills).await
    }

    pub async fn rebuild_agent(&self, agent: AgentDefinition) -> Result<(), String> {
        self.send(HarnessCommand::RebuildAgent(agent)).await
    }

    pub async fn set_model(&self, model: ModelDef) -> Result<(), String> {
        self.send(HarnessCommand::SetModel(model)).await
    }

    pub async fn set_agent(&self, agent: AgentDefinition) -> Result<(), String> {
        self.send(HarnessCommand::SetAgent(agent)).await
    }

    pub async fn new_session(&self, cwd: Option<PathBuf>) -> Result<(), String> {
        self.send(HarnessCommand::NewSession(cwd)).await
    }

    pub async fn fork_session(&self) -> Result<(), String> {
        self.send(HarnessCommand::ForkSession).await
    }

    pub async fn load_session(&self, session_id: String) -> Result<(), String> {
        self.send(HarnessCommand::LoadSession(session_id)).await
    }

    pub async fn close_session(&self) -> Result<(), String> {
        self.send(HarnessCommand::CloseSession).await
    }

    pub async fn toggle_streaming(&self) -> Result<(), String> {
        self.send(HarnessCommand::ToggleStreaming).await
    }

    pub async fn toggle_plan_mode(&self) -> Result<(), String> {
        self.send(HarnessCommand::TogglePlanMode).await
    }

    pub async fn replay_history(&self) -> Result<(), String> {
        self.send(HarnessCommand::ReplayHistory).await
    }

    pub async fn compact(&self) -> Result<(), String> {
        self.send(HarnessCommand::Compact).await
    }

    pub async fn cancel(&self) -> Result<(), String> {
        self.send(HarnessCommand::Cancel).await
    }

    pub async fn info(&self) -> Result<(), String> {
        self.send(HarnessCommand::Info).await
    }

    pub async fn shutdown(&self) -> Result<(), String> {
        self.send(HarnessCommand::Shutdown).await
    }
}

pub struct HarnessActorServer {
    harness: Harness,
    rx: Receiver<HarnessCommand>,
    tx: UnboundedSender<HarnessEvent>,
}

impl HarnessActorServer {
    pub fn new(
        harness: Harness,
        rx: Receiver<HarnessCommand>,
        tx: UnboundedSender<HarnessEvent>,
    ) -> Self {
        Self { harness, rx, tx }
    }

    async fn run_turn(&mut self, request: UserRequest) -> Result<(), String> {
        let info_ref = self.harness.session_info();
        let cancel_flag = self.harness.cancel_handle();
        let prompt_queue = self.harness.prompt_queue();

        let on_event = {
            let event_tx = self.tx.clone();
            move |event: HarnessMsg| -> Option<String> {
                match event {
                    HarnessMsg::AskUser { title, options } => {
                        let (answer_tx, answer_rx) = oneshot::channel();
                        let _ = event_tx.send(HarnessEvent::AskUser {
                            title,
                            options,
                            answer_tx,
                        });
                        Some(executor::block_on(answer_rx).unwrap_or_default())
                    }
                    event => {
                        let _ = event_tx.send(HarnessEvent::Message(event));
                        None
                    }
                }
            }
        };

        let busy_tx = self.tx.clone();
        let mut turn = Box::pin(self.harness.run_turn(request, on_event));
        let result: Result<(), String> = loop {
            tokio::select! {
                result = &mut turn => {
                    break result.map_err(|err| err.to_string());
                }
                cmd = self.rx.recv() => {
                    match cmd {
                        Some(HarnessCommand::Cancel) => {
                            cancel_flag.cancel();
                        }
                        Some(HarnessCommand::Info) => {
                            let info = info_ref.lock().unwrap().clone();
                            let _ = self.tx.send(CommandReply::SessionInfo(Ok(info)).into());
                        }
                        Some(HarnessCommand::RunPrompt(prompt)) => {
                            prompt_queue.push(prompt, &self.tx);
                        }
                        Some(_) => Self::send_busy_reply(&busy_tx),
                        None => break Err("Actor channel closed".into()),
                    }
                }
            }
        };

        drop(turn);

        let current = self.harness.snapshot();
        match &result {
            Ok(()) => {
                let _ = self.tx.send(HarnessEvent::TurnFinished(current));
            }
            Err(err) => {
                let _ = self
                    .tx
                    .send(HarnessEvent::RequestFailed(err.clone(), current));
            }
        }

        result
    }

    async fn run_compaction(&mut self) -> Result<(), String> {
        let info_ref = self.harness.session_info();
        let cancel_flag = self.harness.cancel_handle();
        let prompt_queue = self.harness.prompt_queue();
        let event_tx_for_callback = self.tx.clone();
        let mut on_event = move |event: HarnessMsg| -> Option<String> {
            let _ = event_tx_for_callback.send(HarnessEvent::Message(event));
            None
        };

        let busy_tx = self.tx.clone();
        let mut compaction = Box::pin(self.harness.compact(&mut on_event));
        let result: Result<(), String> = loop {
            tokio::select! {
                result = &mut compaction => break result.map_err(|err| err.to_string()),
                cmd = self.rx.recv() => match cmd {
                    Some(HarnessCommand::Cancel) => cancel_flag.cancel(),
                    Some(HarnessCommand::Info) => {
                        let info = info_ref.lock().unwrap().clone();
                        let _ = self.tx.send(CommandReply::SessionInfo(Ok(info)).into());
                    }
                    Some(HarnessCommand::RunPrompt(prompt)) => {
                        prompt_queue.push(prompt, &self.tx);
                    }
                    Some(_) => Self::send_busy_reply(&busy_tx),
                    None => break Err("Actor channel closed".into()),
                },
            }
        };

        drop(compaction);
        let snapshot = self.harness.snapshot();
        match &result {
            Ok(()) => {
                let _ = self.tx.send(HarnessEvent::TurnFinished(snapshot));
            }
            Err(err) => {
                let _ = self
                    .tx
                    .send(HarnessEvent::RequestFailed(err.clone(), snapshot));
            }
        }
        result
    }

    async fn run_actor(&mut self) {
        while let Some(command) = self.rx.recv().await {
            match command {
                HarnessCommand::RunPrompt(prompt) => {
                    let _ = self.run_turn(UserRequest::Prompt(prompt)).await;
                }
                HarnessCommand::RunSkill(name) => {
                    let _ = self.run_turn(UserRequest::Skill(name)).await;
                }
                HarnessCommand::GetSkills => {
                    let skills = self.harness.list_skills();
                    let _ = self.tx.send(CommandReply::SkillsListed(Ok(skills)).into());
                }
                HarnessCommand::ReloadSkills => {
                    let skills = self.harness.reload_skills();
                    let _ = self
                        .tx
                        .send(CommandReply::SkillsReloaded(Ok(skills)).into());
                }
                HarnessCommand::RebuildAgent(agent) => {
                    let name = agent.name.clone();
                    self.harness.reload_agent(agent);
                    let _ = self.tx.send(CommandReply::AgentRebuilt(Ok(name)).into());
                }
                HarnessCommand::SetModel(model) => {
                    let result = self
                        .harness
                        .set_model(model.clone())
                        .await
                        .map(|_| model.clone())
                        .map_err(|err| err.to_string());
                    let _ = self.tx.send(CommandReply::ModelSet(result).into());
                }
                HarnessCommand::SetAgent(agent) => {
                    let name = agent.name.clone();
                    self.harness.set_agent(agent);
                    let _ = self.tx.send(CommandReply::AgentSet(Ok(name)).into());
                }
                HarnessCommand::NewSession(cwd) => {
                    let result = self
                        .harness
                        .new_session(cwd)
                        .map(|_| self.harness.snapshot())
                        .map_err(|err| err.to_string());
                    let _ = self.tx.send(CommandReply::SessionCreated(result).into());
                }
                HarnessCommand::ForkSession => {
                    let result = self
                        .harness
                        .fork_session()
                        .map(|_| self.harness.snapshot())
                        .map_err(|err| err.to_string());
                    let _ = self.tx.send(CommandReply::SessionForked(result).into());
                }
                HarnessCommand::LoadSession(session_id) => {
                    let result = self
                        .harness
                        .load_session_by_id(&session_id)
                        .map(|_| self.harness.snapshot())
                        .map_err(|err| err.to_string());
                    let _ = self.tx.send(CommandReply::SessionLoaded(result).into());
                }
                HarnessCommand::CloseSession => {
                    let result = self
                        .harness
                        .close_session()
                        .map(|_| self.harness.snapshot())
                        .map_err(|err| err.to_string());
                    let _ = self.tx.send(CommandReply::SessionClosed(result).into());
                }
                HarnessCommand::ToggleStreaming => {
                    let streaming = !self.harness.streaming();
                    self.harness.set_streaming(streaming);
                    let _ = self
                        .tx
                        .send(CommandReply::StreamingToggled(Ok(streaming)).into());
                }
                HarnessCommand::TogglePlanMode => {
                    let plan_mode = !self.harness.plan_mode();
                    self.harness.set_plan_mode(plan_mode);
                    let _ = self
                        .tx
                        .send(CommandReply::PlanModeToggled(Ok(plan_mode)).into());
                }
                HarnessCommand::ReplayHistory => {
                    let mut events = Vec::new();
                    self.harness.replay_history(|event| events.push(event));
                    let _ = self.tx.send(HarnessEvent::HistoryReplayed(events));
                }
                HarnessCommand::Compact => {
                    let _ = self.run_compaction().await;
                }
                HarnessCommand::Cancel => {
                    self.harness.set_cancel(true);
                }
                HarnessCommand::Info => {
                    let info = self.harness.session_info();
                    let info = info.lock().unwrap().clone();
                    let _ = self.tx.send(CommandReply::SessionInfo(Ok(info)).into());
                }
                HarnessCommand::Shutdown => {
                    let _ = self
                        .tx
                        .send(HarnessEvent::ShutdownComplete(Ok(self.harness.snapshot())));
                    break;
                }
            }
        }
    }

    fn send_busy_reply(tx: &UnboundedSender<HarnessEvent>) {
        let _ = tx.send(
            CommandReply::Busy("Agent is busy; wait for idle running this command".to_string())
                .into(),
        );
    }
}
