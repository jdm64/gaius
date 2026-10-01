/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{harness::HarnessEvent, harness_actor::HarnessActorEvent};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Default)]
pub struct PromptQueue {
    prompts: Arc<Mutex<VecDeque<String>>>,
}

impl PromptQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, prompt: String, event_tx: &UnboundedSender<HarnessActorEvent>) {
        let mut queue = self.prompts.lock().unwrap();
        queue.push_back(prompt);
        let _ = event_tx.send(HarnessActorEvent::Harness(HarnessEvent::QueueChanged(
            queue.len(),
        )));
    }

    pub fn pop(&self) -> Option<String> {
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
