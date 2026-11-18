/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::state::{PendingPrompts, SessState, SharedSessionId},
    actor::{FinishedReason, HarnessActorClient, HarnessCommand, HarnessEvent},
    harness::HarnessMsg,
};
use agent_client_protocol::{Client, ConnectionTo, Responder};
use tokio::sync::{
    mpsc::{Sender, UnboundedReceiver},
    oneshot,
};

pub trait ACPEventProtocol {
    fn on_prompt(&self, msg_id: u32, resp: Box<Responder>);
    fn on_message(&self, sess_id: String, msg: &HarnessMsg, cx: &ConnectionTo<Client>);
    fn on_ask_user(
        &self,
        sess_id: String,
        call_id: String,
        title: String,
        options: Vec<String>,
        answer_tx: oneshot::Sender<String>,
        cx: &ConnectionTo<Client>,
    );
    fn on_turn_finished(&self, sess_id: String, status: FinishedReason, cx: &ConnectionTo<Client>);
}

pub struct ACPEventHandler {
    events: UnboundedReceiver<HarnessEvent>,
    tx: Sender<HarnessCommand>,
    session_id: SharedSessionId,
    pending_prompts: PendingPrompts,
    protocol: Box<dyn ACPEventProtocol>,
}

impl ACPEventHandler {
    pub fn new(
        events: UnboundedReceiver<HarnessEvent>,
        tx: Sender<HarnessCommand>,
        session_id: SharedSessionId,
        pending_prompts: PendingPrompts,
        protocol: Box<dyn ACPEventProtocol>,
    ) -> Self {
        Self {
            events,
            tx,
            session_id,
            pending_prompts,
            protocol,
        }
    }

    pub async fn run(mut self, cx: ConnectionTo<Client>) {
        loop {
            tokio::select! {
                _ = cx.incoming_closed() => {
                    HarnessActorClient::shutdown(self.tx, &mut self.events).await;
                    break;
                },
                event = self.events.recv() => {
                    let Some(event) = event else { break; };
                    match event {
                        HarnessEvent::Message(msg) => {
                            let Some(session_id) = SessState::get(&self.session_id) else {
                                break;
                            };
                            if let HarnessMsg::UserPrompt { msg_id, .. } = &msg {
                                self.pending_prompts.respond(*msg_id, self.protocol.as_ref()).await;
                            }
                            self.protocol.on_message(session_id, &msg, &cx);
                        }
                        HarnessEvent::AskUser { call_id, title, options, answer_tx } => {
                            let Some(session_id) = SessState::get(&self.session_id) else {
                                let _ = answer_tx.send(String::new());
                                break;
                            };
                            self.protocol.on_ask_user(session_id, call_id, title, options, answer_tx, &cx);
                        }
                        HarnessEvent::TurnFinished { status, ack, .. } => {
                            let Some(session_id) = SessState::get(&self.session_id) else {
                                break;
                            };
                            self.protocol.on_turn_finished(session_id, status, &cx);
                            if let Some(ack) = ack {
                                let _ = ack.send(());
                            }
                        }
                        HarnessEvent::ShutdownComplete(_) => break
                    }
                }
            }
        }
    }
}
