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
    harness::{Harness, HarnessEvent, HarnessSnapshot},
};
use genai::Client;
use tokio::sync::{mpsc, oneshot};

type CommandResult = Result<HarnessSnapshot, String>;

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
        client: Client,
        model: String,
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
    ReplayHistory {
        reply_tx: oneshot::Sender<CommandResult>,
    },
    Shutdown {
        reply_tx: oneshot::Sender<HarnessSnapshot>,
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
        self.tx
            .send(HarnessCommand::RunPrompt(prompt))
            .await
            .map_err(|_| "Harness actor stopped".to_string())
    }

    pub async fn set_model(&self, client: Client, model: String) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(HarnessCommand::SetModel {
                client,
                model,
                reply_tx,
            })
            .await
            .map_err(|_| "Harness actor stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "Harness actor stopped".to_string())?
    }

    pub async fn set_agent(&self, agent: AgentDefinition) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(HarnessCommand::SetAgent { agent, reply_tx })
            .await
            .map_err(|_| "Harness actor stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "Harness actor stopped".to_string())?
    }

    pub async fn new_session(&self) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(HarnessCommand::NewSession { reply_tx })
            .await
            .map_err(|_| "Harness actor stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "Harness actor stopped".to_string())?
    }

    pub async fn load_session(&self, session_id: String) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(HarnessCommand::LoadSession {
                session_id,
                reply_tx,
            })
            .await
            .map_err(|_| "Harness actor stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "Harness actor stopped".to_string())?
    }

    pub async fn toggle_streaming(&self) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(HarnessCommand::ToggleStreaming { reply_tx })
            .await
            .map_err(|_| "Harness actor stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "Harness actor stopped".to_string())?
    }

    pub async fn replay_history(&self) -> CommandResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(HarnessCommand::ReplayHistory { reply_tx })
            .await
            .map_err(|_| "Harness actor stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "Harness actor stopped".to_string())?
    }

    pub async fn shutdown(&self) -> Result<HarnessSnapshot, String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(HarnessCommand::Shutdown { reply_tx })
            .await
            .map_err(|_| "Harness actor stopped".to_string())?;
        reply_rx
            .await
            .map_err(|_| "Harness actor stopped".to_string())
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
                let result: Result<(), String> = harness
                    .run_turn_with_events(prompt, |event| match event {
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
                    })
                    .await
                    .map_err(|err| err.to_string());

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
            HarnessCommand::SetModel {
                client,
                model,
                reply_tx,
            } => {
                harness.set_model(client, model);
                let _ = reply_tx.send(Ok(harness.snapshot()));
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
            HarnessCommand::ReplayHistory { reply_tx } => {
                let mut events = Vec::new();
                harness.replay_history(|event| events.push(event));
                let _ = event_tx.send(HarnessActorEvent::HistoryReplayed(events));
                let _ = reply_tx.send(Ok(harness.snapshot()));
            }
            HarnessCommand::Shutdown { reply_tx } => {
                let _ = reply_tx.send(harness.snapshot());
                break;
            }
        }
    }
}
