/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{acp::engine::ACPEngine, actor::CommandReply, models::Models};
use agent_client_protocol::{Client, ConnectionTo, Error, Responder};

impl ACPEngine {
    pub async fn handle_command(
        &mut self,
        command: &str,
        resp: Responder,
        cx: &ConnectionTo<Client>,
    ) -> Result<(), Error> {
        match command {
            "/models-refresh" => {
                if let Err(error) = self.handle_models_refresh(cx).await {
                    return resp.respond_with_error(error);
                }
            }
            "/info" => {
                if let Err(error) = self.handle_info(cx).await {
                    return resp.respond_with_error(error);
                }
            }
            _ => return resp.respond_with_error(Error::invalid_params()),
        }

        let session_id = match self.sess_state.get_id() {
            Some(session_id) => session_id,
            None => return resp.respond_with_error(Error::internal_error()),
        };
        self.protocol.send_command_finished(session_id, resp, cx)
    }

    async fn handle_info(&mut self, cx: &ConnectionTo<Client>) -> Result<(), Error> {
        if self.actor.info().await.is_err() {
            return Err(Error::internal_error());
        }
        let info = match self.replies.recv().await {
            Some(CommandReply::SessionInfo(Ok(info))) => info,
            _ => return Err(Error::internal_error()),
        };

        let value = serde_json::to_value(info.usage).map_err(|_| Error::internal_error())?;
        let content = value
            .as_object()
            .into_iter()
            .flat_map(|object| object.iter())
            .filter(|&(_key, value)| !value.is_null())
            .map(|(key, value)| {
                format!(
                    "{key}: {}",
                    value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let session_id = self.sess_state.get_id().ok_or_else(Error::internal_error)?;
        self.protocol.send_agent_msg(&session_id, content, cx);
        Ok(())
    }

    async fn handle_models_refresh(&mut self, cx: &ConnectionTo<Client>) -> Result<(), Error> {
        let session_id = match self.sess_state.get_id() {
            Some(session_id) => session_id,
            None => return Err(Error::internal_error()),
        };
        match Models::reload(&self.config).await {
            Ok(models) => {
                let before = self.models.len();
                self.models = models;
                let after = self.models.len();
                let content = format!("Refreshed models: {} total, {} new", after, after - before);
                self.protocol.send_agent_msg(&session_id, content, cx);
            }
            Err(_) => return Err(Error::internal_error()),
        }
        self.protocol.send_config_update(self, &session_id, cx)
    }
}
