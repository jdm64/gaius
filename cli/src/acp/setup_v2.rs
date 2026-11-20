/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{
        engine::ACPEngine, event::ACPEventHandler, protocol_v2::ProtocolV2, setup::ConnectionState,
    },
    config::Config,
};
use agent_client_protocol::{
    Agent, Client, ConnectTo, ConnectionTo, Dispatch, Error, HandleDispatchFrom, Handled,
    v2::schema::{ClientNotification, ClientRequest},
};
use core::fmt::Debug;
use std::sync::Arc;

pub struct ACPSetupV2 {
    config: Arc<Config>,
}

impl ACPSetupV2 {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

impl ConnectTo<Client> for ACPSetupV2 {
    async fn connect_to(self, client: impl ConnectTo<Agent>) -> Result<(), Error> {
        let state = ConnectionState::build(self.config, Box::new(ProtocolV2)).await?;

        Agent
            .v2()
            .name("gaius")
            .with_handler(ACPDispatchV2::new(state.engine))
            .connect_with(client, async move |cx| {
                let handler = ACPEventHandler::new(
                    state.events,
                    state.actor_tx,
                    state.session_id,
                    state.prompts,
                    Box::new(ProtocolV2),
                );
                handler.run(cx.into()).await;
                Ok(())
            })
            .await
    }
}

pub struct ACPDispatchV2 {
    engine: ACPEngine,
}

impl ACPDispatchV2 {
    pub fn new(engine: ACPEngine) -> Self {
        Self { engine }
    }
}

impl HandleDispatchFrom<Client> for ACPDispatchV2 {
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
        "Gaius (ACP v2)"
    }
}
