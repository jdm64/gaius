/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::actor::CommandReply;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone)]
pub struct PromptQueue {
    prompts: Arc<Mutex<VecDeque<(u32, String)>>>,
    msg_id_counter: Arc<AtomicU32>,
}

impl PromptQueue {
    pub fn new(msg_id_counter: Arc<AtomicU32>) -> Self {
        Self {
            prompts: Arc::new(Mutex::new(VecDeque::new())),
            msg_id_counter,
        }
    }

    pub fn push(&self, prompt: String, reply_tx: &UnboundedSender<CommandReply>) -> u32 {
        let msg_id = self.msg_id_counter.fetch_add(1, Ordering::Relaxed);
        let mut queue = self.prompts.lock().unwrap();
        queue.push_back((msg_id, prompt));
        let _ = reply_tx.send(CommandReply::PromptQueued {
            size: queue.len(),
            msg_id,
        });
        msg_id
    }

    pub fn pop(&self) -> Option<(u32, String)> {
        self.prompts.lock().unwrap().pop_front()
    }

    pub fn len(&self) -> usize {
        self.prompts.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) -> usize {
        let mut prompts = self.prompts.lock().unwrap();
        let dropped = prompts.len();
        prompts.clear();
        dropped
    }
}
