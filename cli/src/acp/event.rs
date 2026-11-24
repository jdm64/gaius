/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{
        protocol::ACPProtocol,
        state::{PendingPrompts, SessState, SharedSessionId},
    },
    actor::{HarnessActorClient, HarnessCommand, HarnessEvent},
    harness::HarnessMsg,
};
use agent_client_protocol::{Client, ConnectionTo};
use tokio::sync::mpsc::{Sender, UnboundedReceiver};

pub struct ACPEventHandler {
    events: UnboundedReceiver<HarnessEvent>,
    tx: Sender<HarnessCommand>,
    session_id: SharedSessionId,
    prompts: PendingPrompts,
    protocol: Box<dyn ACPProtocol>,
}

impl ACPEventHandler {
    pub fn new(
        events: UnboundedReceiver<HarnessEvent>,
        tx: Sender<HarnessCommand>,
        session_id: SharedSessionId,
        prompts: PendingPrompts,
        protocol: Box<dyn ACPProtocol>,
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
                    let _ = HarnessActorClient::shutdown(self.tx, &mut self.events).await;
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
                let Some(session_id) = SessState::id(&self.session_id) else {
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
                let Some(session_id) = SessState::id(&self.session_id) else {
                    let _ = answer_tx.send(String::new());
                    return;
                };
                self.protocol
                    .on_ask_user(session_id, call_id, title, options, answer_tx, cx);
            }
            HarnessEvent::TurnFinished { status, ack, .. } => {
                let Some(session_id) = SessState::id(&self.session_id) else {
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
