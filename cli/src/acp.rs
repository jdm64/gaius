/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

pub mod engine;
pub mod event_handler;
pub mod event_protocol_v2;
pub mod message;
pub mod message_v2;
pub mod protocol_v2;
pub mod state;

use crate::{acp::protocol_v2::ACPSetupV2, config::Config};
use agent_client_protocol::{Agent, ConnectTo, Error, JsonRpcResponse, Responder, Stdio};
use core::error::Error as CoreError;
use serde_json::Value;
use std::sync::Arc;

pub fn reply<T: JsonRpcResponse>(r: Responder<Value>, resp: T) -> Result<(), Error> {
    r.cast::<T>().respond(resp)
}

pub async fn run_acp() -> Result<(), Box<dyn CoreError>> {
    let mut config = Config::new();
    config.load().await?;
    let config = Arc::new(config);

    Agent
        .protocol_router()
        .with_v2(ACPSetupV2::new(config))
        .connect_to(Stdio::default())
        .await?;

    Ok(())
}
