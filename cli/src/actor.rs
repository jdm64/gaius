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
    time::Duration,
};
use tokio::{
    runtime::Builder,
    sync::{
        mpsc::{self, Receiver, Sender, UnboundedReceiver, UnboundedSender},
        oneshot,
    },
    time,
};

pub type OperationResult<T> = Result<T, String>;
pub type AnswerSender = oneshot::Sender<String>;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const ACTOR_STOPPED: &str = "Harness actor stopped";

#[derive(Debug)]
pub enum HarnessEvent {
    Message(HarnessMsg),
    AskUser {
        call_id: String,
        title: String,
        options: Vec<String>,
        answer_tx: AnswerSender,
    },
    TurnFinished {
        snapshot: HarnessSnapshot,
        status: Result<(), String>,
    },
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
    PromptQueued { size: usize, msg_id: u32 },
    HistoryReplayed(Vec<HarnessMsg>),
    StreamingToggled(OperationResult<bool>),
    PlanModeToggled(OperationResult<bool>),
    SkillsListed(OperationResult<Vec<Skill>>),
    SkillsReloaded(OperationResult<Vec<Skill>>),
    SessionInfo(OperationResult<SessionInfo>),
    Busy(String),
}

pub struct HarnessRxHandle {
    pub events: UnboundedReceiver<HarnessEvent>,
    pub replies: UnboundedReceiver<CommandReply>,
}

pub struct HarnessActorClient {
    tx: Sender<HarnessCommand>,
}

impl HarnessActorClient {
    pub fn new(harness: Harness) -> io::Result<(HarnessActorClient, HarnessRxHandle)> {
        let (tx, rx) = mpsc::channel(64);
        let (event_tx, events) = mpsc::unbounded_channel();
        let (reply_tx, replies) = mpsc::unbounded_channel();

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
                let mut server = HarnessActorServer::new(harness, rx, event_tx, reply_tx);
                rt.block_on(server.run_actor());
            })?;

        ready_rx
            .recv()
            .map_err(|_| Error::other("harness actor thread panicked during startup"))??;

        Ok((
            HarnessActorClient { tx },
            HarnessRxHandle { events, replies },
        ))
    }

    pub fn clone_tx(&self) -> Sender<HarnessCommand> {
        self.tx.clone()
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

    pub async fn shutdown(
        tx: Sender<HarnessCommand>,
        rx: &mut UnboundedReceiver<HarnessEvent>,
    ) -> Option<HarnessSnapshot> {
        match tx.send(HarnessCommand::Shutdown).await {
            Ok(_) => {}
            Err(_) => return None,
        };

        let shutdown = async {
            while let Some(event) = rx.recv().await {
                if let HarnessEvent::ShutdownComplete(result) = event {
                    return result.ok();
                }
            }
            None
        };

        time::timeout(SHUTDOWN_TIMEOUT, shutdown)
            .await
            .ok()
            .flatten()
    }
}

pub struct HarnessActorServer {
    harness: Harness,
    rx: Receiver<HarnessCommand>,
    event_tx: UnboundedSender<HarnessEvent>,
    reply_tx: UnboundedSender<CommandReply>,
}

impl HarnessActorServer {
    pub fn new(
        harness: Harness,
        rx: Receiver<HarnessCommand>,
        event_tx: UnboundedSender<HarnessEvent>,
        reply_tx: UnboundedSender<CommandReply>,
    ) -> Self {
        Self {
            harness,
            rx,
            event_tx,
            reply_tx,
        }
    }

    fn tx<T>(sender: &UnboundedSender<T>, msg: T) {
        let _ = sender.send(msg);
    }

    fn tx_busy(tx: &UnboundedSender<CommandReply>) {
        let _ = tx.send(CommandReply::Busy(
            "Agent is busy; wait for idle before running this command".to_string(),
        ));
    }

    async fn run_turn(&mut self, request: UserRequest) -> Result<(), String> {
        let info_ref = self.harness.session_info();
        let cancel_flag = self.harness.cancel_handle();
        let prompt_queue = self.harness.prompt_queue();

        let on_event = {
            let event_tx = self.event_tx.clone();
            move |event: HarnessMsg| -> Option<String> {
                match event {
                    HarnessMsg::AskUser { call_id, title, options } => {
                        let (answer_tx, answer_rx) = oneshot::channel();
                        Self::tx(
                            &event_tx,
                            HarnessEvent::AskUser {
                                call_id,
                                title,
                                options,
                                answer_tx,
                            },
                        );
                        Some(executor::block_on(answer_rx).unwrap_or_default())
                    }
                    event => {
                        Self::tx(&event_tx, HarnessEvent::Message(event));
                        None
                    }
                }
            }
        };

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
                            Self::tx(&self.reply_tx, CommandReply::SessionInfo(Ok(info)));
                        }
                        Some(HarnessCommand::RunPrompt(prompt)) => {
                            prompt_queue.push(prompt, &self.reply_tx);
                        }
                        Some(_) => Self::tx_busy(&self.reply_tx),
                        None => break Err("Actor channel closed".into()),
                    }
                }
            }
        };

        drop(turn);

        Self::tx(
            &self.event_tx,
            HarnessEvent::TurnFinished {
                snapshot: self.harness.snapshot(),
                status: result.clone(),
            },
        );

        result
    }

    async fn run_compaction(&mut self) -> Result<(), String> {
        let info_ref = self.harness.session_info();
        let cancel_flag = self.harness.cancel_handle();
        let prompt_queue = self.harness.prompt_queue();
        let event_tx_for_callback = self.event_tx.clone();
        let mut on_event = move |event: HarnessMsg| -> Option<String> {
            Self::tx(&event_tx_for_callback, HarnessEvent::Message(event));
            None
        };

        let mut compaction = Box::pin(self.harness.compact(&mut on_event));
        let result: Result<(), String> = loop {
            tokio::select! {
                result = &mut compaction => break result.map_err(|err| err.to_string()),
                cmd = self.rx.recv() => match cmd {
                    Some(HarnessCommand::Cancel) => cancel_flag.cancel(),
                    Some(HarnessCommand::Info) => {
                        let info = info_ref.lock().unwrap().clone();
                        Self::tx(&self.reply_tx, CommandReply::SessionInfo(Ok(info)));
                    }
                    Some(HarnessCommand::RunPrompt(prompt)) => {
                        prompt_queue.push(prompt, &self.reply_tx);
                    }
                    Some(_) => Self::tx_busy(&self.reply_tx),
                    None => break Err("Actor channel closed".into()),
                },
            }
        };

        drop(compaction);

        Self::tx(
            &self.event_tx,
            HarnessEvent::TurnFinished {
                snapshot: self.harness.snapshot(),
                status: result.clone(),
            },
        );

        result
    }

    async fn run_actor(&mut self) {
        while let Some(command) = self.rx.recv().await {
            match command {
                HarnessCommand::RunPrompt(prompt) => {
                    let msg_id = self.harness.next_msg_id();
                    Self::tx(
                        &self.reply_tx,
                        CommandReply::PromptQueued { size: 0, msg_id },
                    );
                    let _ = self.run_turn(UserRequest::Prompt { msg_id, prompt }).await;
                }
                HarnessCommand::RunSkill(name) => {
                    let _ = self.run_turn(UserRequest::Skill(name)).await;
                }
                HarnessCommand::GetSkills => {
                    let skills = self.harness.list_skills();
                    Self::tx(&self.reply_tx, CommandReply::SkillsListed(Ok(skills)));
                }
                HarnessCommand::ReloadSkills => {
                    let skills = self.harness.reload_skills();
                    Self::tx(&self.reply_tx, CommandReply::SkillsReloaded(Ok(skills)));
                }
                HarnessCommand::RebuildAgent(agent) => {
                    let name = agent.name.clone();
                    self.harness.reload_agent(agent);
                    Self::tx(&self.reply_tx, CommandReply::AgentRebuilt(Ok(name)));
                }
                HarnessCommand::SetModel(model) => {
                    let result = self
                        .harness
                        .set_model(model.clone())
                        .await
                        .map(|_| model.clone())
                        .map_err(|err| err.to_string());
                    Self::tx(&self.reply_tx, CommandReply::ModelSet(result));
                }
                HarnessCommand::SetAgent(agent) => {
                    let name = agent.name.clone();
                    self.harness.set_agent(agent);
                    Self::tx(&self.reply_tx, CommandReply::AgentSet(Ok(name)));
                }
                HarnessCommand::NewSession(cwd) => {
                    let result = self
                        .harness
                        .new_session(cwd)
                        .map(|_| self.harness.snapshot())
                        .map_err(|err| err.to_string());
                    Self::tx(&self.reply_tx, CommandReply::SessionCreated(result));
                }
                HarnessCommand::ForkSession => {
                    let result = self
                        .harness
                        .fork_session()
                        .map(|_| self.harness.snapshot())
                        .map_err(|err| err.to_string());
                    Self::tx(&self.reply_tx, CommandReply::SessionForked(result));
                }
                HarnessCommand::LoadSession(session_id) => {
                    let result = self
                        .harness
                        .load_session_by_id(&session_id)
                        .map(|_| self.harness.snapshot())
                        .map_err(|err| err.to_string());
                    Self::tx(&self.reply_tx, CommandReply::SessionLoaded(result));
                }
                HarnessCommand::CloseSession => {
                    let result = self
                        .harness
                        .close_session()
                        .map(|_| self.harness.snapshot())
                        .map_err(|err| err.to_string());
                    Self::tx(&self.reply_tx, CommandReply::SessionClosed(result));
                }
                HarnessCommand::ToggleStreaming => {
                    let streaming = !self.harness.streaming();
                    self.harness.set_streaming(streaming);
                    Self::tx(
                        &self.reply_tx,
                        CommandReply::StreamingToggled(Ok(streaming)),
                    );
                }
                HarnessCommand::TogglePlanMode => {
                    let plan_mode = !self.harness.plan_mode();
                    self.harness.set_plan_mode(plan_mode);
                    Self::tx(&self.reply_tx, CommandReply::PlanModeToggled(Ok(plan_mode)));
                }
                HarnessCommand::ReplayHistory => {
                    let mut events = Vec::new();
                    self.harness.replay_history(|event| events.push(event));
                    Self::tx(&self.reply_tx, CommandReply::HistoryReplayed(events));
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
                    Self::tx(&self.reply_tx, CommandReply::SessionInfo(Ok(info)));
                }
                HarnessCommand::Shutdown => {
                    self.rx.close();
                    Self::tx(
                        &self.event_tx,
                        HarnessEvent::ShutdownComplete(Ok(self.harness.snapshot())),
                    );
                    break;
                }
            }
        }
    }
}
