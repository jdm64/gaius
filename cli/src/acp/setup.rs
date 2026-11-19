/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    acp::{
        engine::ACPEngine,
        protocol::ACPProtocol,
        state::{PendingPrompts, SessState, SharedSessionId},
    },
    actor::{HarnessActorClient, HarnessCommand, HarnessEvent, HarnessRxHandle},
    config::Config,
    harness::Harness,
    models::Models,
};
use agent_client_protocol::Error;
use core::error::Error as CoreError;
use tokio::sync::mpsc::{Sender, UnboundedReceiver};

pub struct ConnectionState {
    pub engine: ACPEngine,
    pub events: UnboundedReceiver<HarnessEvent>,
    pub actor_tx: Sender<HarnessCommand>,
    pub session_id: SharedSessionId,
    pub prompts: PendingPrompts,
}

impl ConnectionState {
    async fn create_client(
        config: &Config,
    ) -> Result<(HarnessActorClient, HarnessRxHandle), Box<dyn CoreError>> {
        let mut harness = Harness::new(config, None)?;

        let models_cache = Models::list(config).await?;
        if let Some(recent_model) = Models::first_from_recent(&models_cache) {
            harness.set_model(recent_model).await?;
        } else {
            let config_model = Models::first_from_config(config, &models_cache).ok();
            if let Some(config_model) = config_model {
                harness.set_model(config_model).await?;
            }
        }
        Ok(HarnessActorClient::new(harness)?)
    }

    pub async fn build(
        config: &Config,
        protocol: Box<dyn ACPProtocol>,
    ) -> Result<ConnectionState, Error> {
        let (actor, actor_rx) = Self::create_client(config)
            .await
            .map_err(|err| Error::internal_error().data(err.to_string()))?;
        let sess_state = SessState::default();
        let prompts = PendingPrompts::default();
        let engine = ACPEngine {
            actor,
            replies: actor_rx.replies,
            sess_state,
            prompts,
            initialized: false,
            protocol,
        };
        let actor_tx = engine.actor.clone_tx();
        let session_id = engine.sess_state.clone_active();
        let prompts = engine.prompts.clone();

        Ok(ConnectionState {
            engine,
            events: actor_rx.events,
            actor_tx,
            session_id,
            prompts,
        })
    }
}
