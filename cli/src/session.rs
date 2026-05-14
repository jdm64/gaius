/* Copyright 2026 Justin Madru (justin.jdm64@gmail.com)
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use genai::chat::{ChatMessage, ChatRequest};
use rmp_serde::{Deserializer, Serializer};
use serde::{Deserialize, Serialize};
use std::{error::Error, fs::File, io::BufReader, path::PathBuf};
use uuid::Uuid;

fn session_dir() -> Result<PathBuf, Box<dyn Error>> {
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("gaius")
        .join("sessions"))
}

fn session_file(session_id: &str) -> Result<PathBuf, Box<dyn Error>> {
    validate_session_id(session_id)?;
    Ok(session_dir()?.join(format!("{}.mpk", session_id)))
}

fn validate_session_id(session_id: &str) -> Result<(), Box<dyn Error>> {
    if session_id.is_empty() {
        return Err("Session id cannot be empty".into());
    }

    if session_id.contains('/') || session_id.contains('\\') {
        return Err("Session id cannot contain path separators".into());
    }

    Ok(())
}

pub struct Session {
    pub id: Option<String>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: Some(Uuid::now_v7().to_string()),
        }
    }

    pub fn new_named(name: String) -> Result<Self, Box<dyn Error>> {
        validate_session_id(name.as_str())?;
        Ok(Self { id: Some(name) })
    }

    pub fn new_empty() -> Self {
        Self { id: None }
    }

    pub fn load(&self) -> Result<ChatRequest, Box<dyn Error>> {
        let Some(session_id) = self.id.clone() else {
            return Ok(ChatRequest::new(vec![]));
        };

        let path = session_file(session_id.as_str())?;
        if path.is_file() {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            let mut des = Deserializer::new(reader);
            let _version: i32 = Deserialize::deserialize(&mut des)?;
            let messages: Vec<ChatMessage> = Deserialize::deserialize(&mut des)?;
            Ok(ChatRequest::new(messages))
        } else {
            Ok(ChatRequest::new(vec![]))
        }
    }

    pub fn save(&self, history: &ChatRequest) -> Result<(), Box<dyn Error>> {
        if let Some(session_id) = self.id.clone() {
            let path = session_file(session_id.as_str())?;
            let file = File::create(path)?;
            let mut ser = Serializer::new(file).with_struct_map();
            1.serialize(&mut ser)?;
            history.messages.serialize(&mut ser)?;
        }

        Ok(())
    }

    pub fn list() -> Vec<String> {
        let dir = match session_dir() {
            Ok(d) => d,
            Err(_) => return Vec::new(),
        };

        let mut sessions: Vec<String> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .filter_map(|entry| {
                        let path = entry.path();
                        if path.extension()? == "mpk" {
                            path.file_stem()?.to_str().map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        sessions.sort();
        sessions
    }

    pub fn delete(session_id: &str) -> Result<(), Box<dyn Error>> {
        validate_session_id(session_id)?;
        let path = session_file(session_id)?;
        if path.is_file() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
