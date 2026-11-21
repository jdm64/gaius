/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{acp::protocol::ACPProtocol, session::Session};
use agent_client_protocol::Responder;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;

pub type SharedSessionId = Arc<Mutex<Option<String>>>;

#[derive(Default, Clone)]
pub struct SessState {
    active: SharedSessionId,
    known: HashSet<String>,
    cwd: Option<PathBuf>,
}

impl SessState {
    pub fn clone_active(&self) -> SharedSessionId {
        self.active.clone()
    }

    pub fn get_id(shared: &SharedSessionId) -> Option<String> {
        shared.lock().unwrap().clone()
    }

    pub fn get_cwd(&self) -> Option<PathBuf> {
        self.cwd.clone()
    }

    pub fn set(&mut self, sess_id: &str, cwd: PathBuf) {
        *self.active.lock().unwrap() = Some(sess_id.to_string());
        self.known.insert(sess_id.to_string());
        self.cwd = Some(cwd);
    }

    pub fn is_active(&self, sess_id: &str) -> bool {
        self.active.lock().unwrap().as_deref() == Some(sess_id)
    }

    pub fn is_unknown(&self, sess_id: &str) -> bool {
        !self.is_active(sess_id) && !self.known.contains(sess_id) && !Session::exists(sess_id)
    }

    pub fn clear(&mut self) {
        *self.active.lock().unwrap() = None;
        self.cwd = None;
    }

    pub fn remove(&mut self, sess_id: &str) {
        self.known.remove(sess_id);
    }
}

pub enum PendingPrompt {
    Waiting(Box<Responder>),
    Cancelled,
}

#[derive(Default, Clone)]
pub struct PendingPrompts {
    arc: Arc<(Mutex<HashMap<u32, PendingPrompt>>, Notify)>,
}

impl PendingPrompts {
    pub fn insert(&self, msg_id: u32, responder: Responder) {
        self.arc
            .0
            .lock()
            .unwrap()
            .insert(msg_id, PendingPrompt::Waiting(Box::new(responder)));
        self.arc.1.notify_waiters();
    }

    pub fn ack_first(&self, response: impl FnOnce(Box<Responder>)) {
        let responders = {
            let mut prompts = self.arc.0.lock().unwrap();
            let first_id = prompts
                .iter()
                .filter_map(|(msg_id, prompt)| {
                    matches!(prompt, PendingPrompt::Waiting(_)).then_some(*msg_id)
                })
                .min();
            let mut responders = Vec::new();
            if let Some(msg_id) = first_id
                && let Some(PendingPrompt::Waiting(responder)) = prompts.remove(&msg_id)
            {
                responders.push(responder);
            }
            prompts.clear();
            responders
        };

        if let Some(responder) = responders.into_iter().next() {
            response(responder);
        }
    }

    pub async fn ack(&self, msg_id: u32, protocol: &dyn ACPProtocol) {
        loop {
            let notified = self.arc.1.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let slot = self.arc.0.lock().unwrap().remove(&msg_id);
            match slot {
                Some(PendingPrompt::Waiting(responder)) => {
                    protocol.ack_prompt(msg_id, responder);
                    return;
                }
                Some(PendingPrompt::Cancelled) => return,
                None => notified.await,
            }
        }
    }

    pub fn cancel(&self, protocol: &dyn ACPProtocol) {
        let responders: Vec<Box<Responder>> = {
            let mut prompts = self.arc.0.lock().unwrap();
            let mut responders = prompts
                .drain()
                .filter_map(|(msg_id, prompt)| match prompt {
                    PendingPrompt::Waiting(responder) => Some((msg_id, responder)),
                    PendingPrompt::Cancelled => None,
                })
                .collect::<Vec<_>>();
            responders.sort_by_key(|(msg_id, _)| *msg_id);
            responders
                .into_iter()
                .map(|(_, responder)| responder)
                .collect()
        };

        protocol.on_cancel(responders);
        self.arc.1.notify_waiters();
    }
}
