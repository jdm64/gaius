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
    models::{ModelDef, Models},
};
use agent_client_protocol::Error;
use core::error::Error as CoreError;
use std::sync::Arc;
use tokio::sync::mpsc::{Sender, UnboundedReceiver};

pub struct ConnectionState {
    pub engine: ACPEngine,
    pub events: UnboundedReceiver<HarnessEvent>,
    pub actor_tx: Sender<HarnessCommand>,
    pub session_id: SharedSessionId,
    pub prompts: PendingPrompts,
}

impl ConnectionState {
    fn default_model(models: &[ModelDef], config: &Config) -> Option<ModelDef> {
        Models::first_from_recent(models).or_else(|| Models::first_from_config(config, models).ok())
    }

    async fn create_client(
        model: &Option<ModelDef>,
        config: &Config,
    ) -> Result<(HarnessActorClient, HarnessRxHandle), Box<dyn CoreError>> {
        let mut harness = Harness::new(config, None)?;
        if let Some(m) = model {
            harness.set_model(m.clone()).await?;
        }
        Ok(HarnessActorClient::new(harness)?)
    }

    pub async fn build(
        config: Arc<Config>,
        protocol: Box<dyn ACPProtocol>,
    ) -> Result<ConnectionState, Error> {
        let models = Models::list(config.as_ref())
            .await
            .map_err(|err| Error::internal_error().data(err.to_string()))?;
        let curr_model = Self::default_model(&models, config.as_ref());
        let (actor, actor_rx) = Self::create_client(&curr_model, config.as_ref())
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
            config: config.clone(),
            models,
            curr_model: curr_model.unwrap_or_default(),
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
