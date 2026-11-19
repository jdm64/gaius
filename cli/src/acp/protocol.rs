/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{acp::state::PendingPrompts, actor::FinishedReason, harness::HarnessMsg};
use agent_client_protocol::{Client, ConnectionTo, Responder};
use tokio::sync::oneshot;

pub trait ACPProtocol: Send + Sync {
    fn is_ack_prompt(&self) -> bool;

    fn ack_prompt(&self, msg_id: u32, resp: Box<Responder>);

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

    fn on_cancel(&self, responders: Vec<Box<Responder>>);

    fn on_turn_finished(
        &self,
        sess_id: String,
        status: FinishedReason,
        pending: &PendingPrompts,
        cx: &ConnectionTo<Client>,
    );
}
