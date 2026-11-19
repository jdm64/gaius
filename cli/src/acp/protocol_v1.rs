/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{
        engine::{ACPEngine, ConnectionState},
        event::ACPEventHandler,
        event_v1::EventProtocolV1,
    },
    config::Config,
};
use agent_client_protocol::{
    Agent, Client, ConnectTo, ConnectionTo, Dispatch, Error, HandleDispatchFrom, Handled,
    v1::schema::{ClientNotification, ClientRequest},
};
use core::fmt::Debug;
use std::sync::Arc;

pub struct ACPSetupV1 {
    config: Arc<Config>,
}

impl ACPSetupV1 {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

impl ConnectTo<Client> for ACPSetupV1 {
    async fn connect_to(self, client: impl ConnectTo<Agent>) -> Result<(), Error> {
        let state = ConnectionState::build(&self.config)?;

        Agent
            .v1()
            .name("gaius")
            .with_handler(ACPDispatchV1::new(state.engine))
            .connect_with(client, async move |cx| {
                let protocol = Box::new(EventProtocolV1);
                let handler = ACPEventHandler::new(
                    state.events,
                    state.actor_tx,
                    state.session_id,
                    state.pending_prompts,
                    protocol,
                );
                handler.run(cx.into()).await;
                Ok(())
            })
            .await
    }
}

pub struct ACPDispatchV1 {
    engine: ACPEngine,
}

impl ACPDispatchV1 {
    pub fn new(engine: ACPEngine) -> Self {
        Self { engine }
    }
}

impl HandleDispatchFrom<Client> for ACPDispatchV1 {
    async fn handle_dispatch_from(
        &mut self,
        message: Dispatch,
        cx: ConnectionTo<Client>,
    ) -> Result<Handled<Dispatch>, Error> {
        let message = match message.into_request::<ClientRequest>()? {
            Ok((req, responder)) => {
                self.engine
                    .handle_request(req.into(), responder, cx)
                    .await?;
                return Ok(Handled::Yes);
            }
            Err(message) => message,
        };

        let message = match message.into_notification::<ClientNotification>()? {
            Ok(notif) => {
                self.engine.handle_notification(notif.into(), cx).await;
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
        "Gaius (ACP v1)"
    }
}
