/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::AgentDefinition,
    harness::{Harness, HarnessEvent, HarnessSnapshot},
    models::ModelDef,
};
use std::sync::atomic::Ordering;
use tokio::sync::{
    mpsc,
    oneshot::{self, error::RecvError},
};

type CommandResult = Result<HarnessSnapshot, String>;

trait CommandReply: Sized {
    type Output;

    fn from_reply(reply: Result<Self, RecvError>) -> Result<Self::Output, String>;
    fn no_reply() -> Self::Output;
}

impl CommandReply for () {
    type Output = ();

    fn from_reply(reply: Result<Self, RecvError>) -> Result<Self::Output, String> {
        reply.map_err(|_| "Harness actor stopped".to_string())
    }

    fn no_reply() -> Self::Output {}
}

impl CommandReply for Result<HarnessSnapshot, String> {
    type Output = HarnessSnapshot;

    fn from_reply(reply: Result<Self, RecvError>) -> Result<Self::Output, String> {
        reply.map_err(|_| "Harness actor stopped".to_string())?
    }

    fn no_reply() -> Self::Output {
        unreachable!("commands without a reply should use `()`")
    }
}

#[derive(Debug)]
pub enum HarnessActorEvent {
    Harness(HarnessEvent),
    AskUser {
        title: String,
        options: Vec<String>,
        answer_tx: oneshot::Sender<String>,
    },
    TurnStarted,
    TurnFinished(HarnessSnapshot),
    RequestFailed(String, HarnessSnapshot),
    HistoryReplayed(Vec<HarnessEvent>),
}

pub enum HarnessCommand {
    RunPrompt(String),
    SetModel {
        model: ModelDef,
        reply_tx: oneshot::Sender<CommandResult>,
    },
    SetAgent {
        agent: AgentDefinition,
        reply_tx: oneshot::Sender<CommandResult>,
    },
    NewSession {
        reply_tx: oneshot::Sender<CommandResult>,
    },
    LoadSession {
        session_id: String,
        reply_tx: oneshot::Sender<CommandResult>,
    },
    ToggleStreaming {
        reply_tx: oneshot::Sender<CommandResult>,
    },
    TogglePlanMode {
        reply_tx: oneshot::Sender<CommandResult>,
    },
    ReplayHistory {
        reply_tx: oneshot::Sender<CommandResult>,
    },
    Cancel,
    Shutdown {
        reply_tx: oneshot::Sender<CommandResult>,
    },
}

pub struct HarnessActorHandle {
    tx: mpsc::Sender<HarnessCommand>,
    pub rx: mpsc::UnboundedReceiver<HarnessActorEvent>,
}

impl HarnessActorHandle {
    pub fn new(harness: Harness) -> HarnessActorHandle {
        let (tx, command_rx) = mpsc::channel(64);
        let (event_tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run_actor(harness, command_rx, event_tx));

        HarnessActorHandle { tx, rx }
    }

    pub async fn run_prompt(&self, prompt: String) -> Result<(), String> {
        self.send_command::<()>(HarnessCommand::RunPrompt(prompt), None)
            .await
    }

    pub async fn set_model(&self, model: ModelDef) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send_command(HarnessCommand::SetModel { model, reply_tx }, Some(reply_rx))
            .await
    }

    pub async fn set_agent(&self, agent: AgentDefinition) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send_command(HarnessCommand::SetAgent { agent, reply_tx }, Some(reply_rx))
            .await
    }

    pub async fn new_session(&self) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send_command(HarnessCommand::NewSession { reply_tx }, Some(reply_rx))
            .await
    }

    pub async fn load_session(&self, session_id: String) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send_command(
            HarnessCommand::LoadSession {
                session_id,
                reply_tx,
            },
            Some(reply_rx),
        )
        .await
    }

    pub async fn toggle_streaming(&self) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send_command(HarnessCommand::ToggleStreaming { reply_tx }, Some(reply_rx))
            .await
    }

    pub async fn toggle_plan_mode(&self) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send_command(HarnessCommand::TogglePlanMode { reply_tx }, Some(reply_rx))
            .await
    }

    pub async fn replay_history(&self) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send_command(HarnessCommand::ReplayHistory { reply_tx }, Some(reply_rx))
            .await
    }

    pub async fn cancel(&self) -> Result<(), String> {
        self.send_command::<()>(HarnessCommand::Cancel, None).await
    }

    pub async fn shutdown(&self) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send_command(HarnessCommand::Shutdown { reply_tx }, Some(reply_rx))
            .await
    }

    async fn send_command<T>(
        &self,
        command: HarnessCommand,
        reply_rx: Option<oneshot::Receiver<T>>,
    ) -> Result<T::Output, String>
    where
        T: CommandReply,
    {
        self.tx
            .send(command)
            .await
            .map_err(|_| "Harness actor stopped".to_string())?;

        if let Some(reply_rx) = reply_rx {
            T::from_reply(reply_rx.await)
        } else {
            Ok(T::no_reply())
        }
    }
}

async fn run_actor(
    mut harness: Harness,
    mut command_rx: mpsc::Receiver<HarnessCommand>,
    event_tx: mpsc::UnboundedSender<HarnessActorEvent>,
) {
    while let Some(command) = command_rx.recv().await {
        match command {
            HarnessCommand::RunPrompt(prompt) => {
                let _ = event_tx.send(HarnessActorEvent::TurnStarted);
                let cancel_flag = harness.cancel_handle();
                let on_event = {
                    let event_tx = event_tx.clone();
                    move |event: HarnessEvent| -> Option<String> {
                        match event {
                            HarnessEvent::AskUser { title, options } => {
                                let (answer_tx, answer_rx) = oneshot::channel();
                                let _ = event_tx.send(HarnessActorEvent::AskUser {
                                    title,
                                    options,
                                    answer_tx,
                                });
                                Some(futures::executor::block_on(answer_rx).unwrap_or_default())
                            }
                            event => {
                                let _ = event_tx.send(HarnessActorEvent::Harness(event));
                                None
                            }
                        }
                    }
                };

                let mut turn = Box::pin(harness.run_turn_with_events(prompt, on_event));
                let result: Result<(), String> = loop {
                    tokio::select! {
                        result = &mut turn => {
                            break result.map_err(|err| err.to_string());
                        }
                        cmd = command_rx.recv() => {
                            match cmd {
                                Some(HarnessCommand::Cancel) => {
                                    cancel_flag.store(true, Ordering::Relaxed);
                                }
                                Some(_) => {}
                                None => break Err("Actor channel closed".into()),
                            }
                        }
                    }
                };

                drop(turn);

                let current = harness.snapshot();
                match result {
                    Ok(()) => {
                        let _ = event_tx.send(HarnessActorEvent::TurnFinished(current));
                    }
                    Err(err) => {
                        let _ = event_tx.send(HarnessActorEvent::RequestFailed(err, current));
                    }
                }
            }
            HarnessCommand::SetModel { model, reply_tx } => {
                let result = harness
                    .set_model(model)
                    .await
                    .map(|_| harness.snapshot())
                    .map_err(|err| err.to_string());
                let _ = reply_tx.send(result);
            }
            HarnessCommand::SetAgent { agent, reply_tx } => {
                harness.set_agent(agent);
                let _ = reply_tx.send(Ok(harness.snapshot()));
            }
            HarnessCommand::NewSession { reply_tx } => {
                let result = harness
                    .new_session()
                    .map(|_| harness.snapshot())
                    .map_err(|err| err.to_string());
                let _ = reply_tx.send(result);
            }
            HarnessCommand::LoadSession {
                session_id,
                reply_tx,
            } => {
                let result = harness
                    .load_session_by_id(&session_id)
                    .map(|_| harness.snapshot())
                    .map_err(|err| err.to_string());
                let _ = reply_tx.send(result);
            }
            HarnessCommand::ToggleStreaming { reply_tx } => {
                harness.set_streaming(!harness.streaming());
                let _ = reply_tx.send(Ok(harness.snapshot()));
            }
            HarnessCommand::TogglePlanMode { reply_tx } => {
                harness.set_plan_mode(!harness.plan_mode());
                let _ = reply_tx.send(Ok(harness.snapshot()));
            }
            HarnessCommand::ReplayHistory { reply_tx } => {
                let mut events = Vec::new();
                harness.replay_history(|event| events.push(event));
                let _ = event_tx.send(HarnessActorEvent::HistoryReplayed(events));
                let _ = reply_tx.send(Ok(harness.snapshot()));
            }
            HarnessCommand::Cancel => {
                harness.set_cancel(true);
            }
            HarnessCommand::Shutdown { reply_tx } => {
                let _ = reply_tx.send(Ok(harness.snapshot()));
                break;
            }
        }
    }
}
