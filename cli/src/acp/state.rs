/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{acp::util::reply, session::Session};
use agent_client_protocol::{Error, Responder, v2::schema::PromptResponse};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;

pub type SharedSessionId = Arc<Mutex<Option<String>>>;

#[derive(Default, Clone)]
pub struct SessState {
    active: SharedSessionId,
    known: HashSet<String>,
}

impl SessState {
    pub fn clone_active(&self) -> SharedSessionId {
        self.active.clone()
    }

    pub fn get(shared: &SharedSessionId) -> Option<String> {
        shared.lock().unwrap().clone()
    }

    pub fn set(&mut self, sess_id: &str) {
        *self.active.lock().unwrap() = Some(sess_id.to_string());
        self.known.insert(sess_id.to_string());
    }

    pub fn is_active(&self, sess_id: &str) -> bool {
        self.active.lock().unwrap().as_deref() == Some(sess_id)
    }

    pub fn is_unknown(&self, sess_id: &str) -> bool {
        !self.is_active(sess_id) && !self.known.contains(sess_id) && !Session::exists(sess_id)
    }

    pub fn clear(&mut self) {
        *self.active.lock().unwrap() = None;
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

    pub async fn respond(&self, msg_id: u32) {
        loop {
            let notified = self.arc.1.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let slot = self.arc.0.lock().unwrap().remove(&msg_id);
            match slot {
                Some(PendingPrompt::Waiting(responder)) => {
                    let _ = reply(*responder, PromptResponse::new(msg_id.to_string()));
                    return;
                }
                Some(PendingPrompt::Cancelled) => return,
                None => notified.await,
            }
        }
    }

    pub fn cancel(&self) {
        let responders: Vec<Box<Responder>> = {
            let mut prompts = self.arc.0.lock().unwrap();
            prompts
                .values_mut()
                .filter_map(
                    |slot| match std::mem::replace(slot, PendingPrompt::Cancelled) {
                        PendingPrompt::Waiting(responder) => Some(responder),
                        PendingPrompt::Cancelled => None,
                    },
                )
                .collect()
        };
        for responder in responders {
            let _ = responder.respond_with_error(Error::request_cancelled());
        }
        self.arc.1.notify_waiters();
    }
}
