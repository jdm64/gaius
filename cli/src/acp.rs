/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

pub mod engine;
pub mod event_handler;
pub mod event_protocol_v2;
pub mod message;
pub mod message_v2;
pub mod state;

use crate::acp::{
    engine::{ACPEngine, ACPSetup},
    event_handler::ACPEventHandler,
    event_protocol_v2::EventProtocolV2,
};
use agent_client_protocol::{
    Agent, Client, ConnectionTo, Dispatch, Error, HandleDispatchFrom, Handled, JsonRpcResponse,
    Responder, Stdio,
    v2::schema::{ClientNotification, ClientRequest},
};
use core::error::Error as CoreError;
use serde_json::Value;
use std::fmt::Debug;

pub fn reply<T: JsonRpcResponse>(r: Responder<Value>, resp: T) -> Result<(), Error> {
    r.cast::<T>().respond(resp)
}

impl HandleDispatchFrom<Client> for ACPEngine {
    async fn handle_dispatch_from(
        &mut self,
        message: Dispatch,
        cx: ConnectionTo<Client>,
    ) -> Result<Handled<Dispatch>, Error> {
        let message = match message.into_request::<ClientRequest>()? {
            Ok((req, responder)) => {
                self.handle_request(req.into(), responder, cx).await?;
                return Ok(Handled::Yes);
            }
            Err(message) => message,
        };

        let message = match message.into_notification::<ClientNotification>()? {
            Ok(notif) => {
                self.handle_notification(notif.into(), cx).await;
                return Ok(Handled::Yes);
            }
            Err(message) => message,
        };

        Ok(Handled::No {
            message,
            retry: false,
        })
    }

    fn describe_chain(&self) -> impl Debug {
        "Gaius"
    }
}

pub async fn run_acp() -> Result<(), Box<dyn CoreError>> {
    let ACPSetup { engine, events } = match ACPEngine::setup().await {
        Ok(setup) => setup,
        Err(error) => {
            return Err(error);
        }
    };
    let actor_tx = engine.actor.clone_tx();
    let session_id = engine.sess_state.clone_active();
    let pending_prompts = engine.pending_prompts.clone();

    Agent
        .v2()
        .name("gaius")
        .with_handler(engine)
        .connect_with(Stdio::default(), async move |cx| {
            let protocol = Box::new(EventProtocolV2 {});
            ACPEventHandler::new(events, actor_tx, session_id, pending_prompts, protocol)
                .run(cx.into())
                .await;
            Ok(())
        })
        .await?;
    Ok(())
}
