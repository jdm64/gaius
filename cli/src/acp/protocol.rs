/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{engine::ACPEngine, state::PendingPrompts},
    actor::{AnswerSender, FinishedReason},
    harness::HarnessMsg,
};
use agent_client_protocol::{Client, ConnectionTo, Error, Responder};

pub trait ACPProtocol: Send + Sync {
    fn send_error_notice(
        &self,
        title: String,
        msg: String,
        sess_id: String,
        cx: &ConnectionTo<Client>,
    );

    fn send_config_update(
        &self,
        engine: &ACPEngine,
        session_id: &str,
        cx: &ConnectionTo<Client>,
    ) -> Result<(), Error>;

    fn send_commands(&self, session_id: &str, cx: &ConnectionTo<Client>) -> Result<(), Error>;

    fn send_agent_msg(&self, sess_id: &str, content: String, cx: &ConnectionTo<Client>);

    fn send_command_finished(
        &self,
        sess_id: String,
        resp: Responder,
        cx: &ConnectionTo<Client>,
    ) -> Result<(), Error>;

    fn is_ack_prompt(&self) -> bool;

    fn ack_prompt(&self, msg_id: u32, resp: Box<Responder>);

    fn on_message(&self, sess_id: String, msg: &HarnessMsg, cx: &ConnectionTo<Client>);

    fn on_ask_user(
        &self,
        sess_id: String,
        call_id: String,
        title: String,
        options: Vec<String>,
        answer_tx: AnswerSender,
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
