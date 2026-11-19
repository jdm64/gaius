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

pub trait ACPEventProtocol: Send + Sync {
    fn ack_prompt(&self, msg_id: u32, resp: Box<Responder>);

    fn is_ack_prompt(&self) -> bool {
        true
    }

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

    fn on_turn_finished(
        &self,
        sess_id: String,
        status: FinishedReason,
        pending: &PendingPrompts,
        cx: &ConnectionTo<Client>,
    );
}

pub struct ACPEventHandler {
    events: UnboundedReceiver<HarnessEvent>,
    tx: Sender<HarnessCommand>,
    session_id: SharedSessionId,
    prompts: PendingPrompts,
    protocol: Box<dyn ACPEventProtocol>,
}

impl ACPEventHandler {
    pub fn new(
        events: UnboundedReceiver<HarnessEvent>,
        tx: Sender<HarnessCommand>,
        session_id: SharedSessionId,
        prompts: PendingPrompts,
        protocol: Box<dyn ACPEventProtocol>,
    ) -> Self {
        Self {
            events,
            tx,
            session_id,
            prompts,
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
                    self.handle_events(event, &cx).await
                }
            }
        }
    }

    async fn handle_events(&self, event: Option<HarnessEvent>, cx: &ConnectionTo<Client>) {
        let Some(event) = event else {
            return;
        };
        match event {
            HarnessEvent::Message(msg) => {
                let Some(session_id) = SessState::get(&self.session_id) else {
                    return;
                };
                if self.protocol.is_ack_prompt()
                    && let HarnessMsg::UserPrompt { msg_id, .. } = &msg
                {
                    self.prompts.ack(*msg_id, self.protocol.as_ref()).await;
                }
                self.protocol.on_message(session_id, &msg, cx);
            }
            HarnessEvent::AskUser {
                call_id,
                title,
                options,
                answer_tx,
            } => {
                let Some(session_id) = SessState::get(&self.session_id) else {
                    let _ = answer_tx.send(String::new());
                    return;
                };
                self.protocol
                    .on_ask_user(session_id, call_id, title, options, answer_tx, cx);
            }
            HarnessEvent::TurnFinished { status, ack, .. } => {
                let Some(session_id) = SessState::get(&self.session_id) else {
                    return;
                };
                self.protocol
                    .on_turn_finished(session_id, status, &self.prompts, cx);
                if let Some(ack) = ack {
                    let _ = ack.send(());
                }
            }
            HarnessEvent::ShutdownComplete(_) => {}
        }
    }
}
