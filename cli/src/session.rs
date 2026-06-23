/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    dirs::Dirs,
    token_usage::{TokenUsageLedger, TokenUsageSpan, UsageInfo},
};
use genai::chat::{ChatMessage, ChatRequest, ChatRole, MessageContent};
use rmp_serde::{Deserializer, Serializer};
use serde::{Deserialize, Serialize};
use std::{error::Error, fs::File, io::BufReader, path::Path};
use uuid::Uuid;

pub struct SessionFile {
    _id: i32,
    name: String,
    messages: Option<Vec<ChatMessage>>,
    token_usage: Option<TokenUsageLedger>,
}

pub struct Session {
    pub id: Option<String>,
    pub name: Option<String>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: Some(Uuid::now_v7().to_string()),
            name: None,
        }
    }

    pub fn new_named(name: String) -> Result<Self, Box<dyn Error>> {
        Dirs::validate_session_id(name.as_str())?;
        Ok(Self {
            id: Some(name),
            name: None,
        })
    }

    pub fn new_empty() -> Self {
        Self {
            id: None,
            name: None,
        }
    }

    fn deserialize_session(path: &Path, read_msgs: bool) -> Result<SessionFile, Box<dyn Error>> {
        let file = File::open(path)?;
        let buf = BufReader::new(file);
        let mut des = Deserializer::new(buf);

        let version: i32 = Deserialize::deserialize(&mut des)?;
        let name: String = Deserialize::deserialize(&mut des)?;
        let mut messages: Option<Vec<ChatMessage>> = None;
        let mut token_usage: Option<TokenUsageLedger> = None;
        if read_msgs {
            messages = Deserialize::deserialize(&mut des)?;
            if version >= 3 {
                token_usage = Some(Deserialize::deserialize(&mut des)?);
            } else if version == 2 {
                let v2: TokenUsageLedgerV2 = Deserialize::deserialize(&mut des)?;
                token_usage = Some(v2.into());
            }
        }

        Ok(SessionFile {
            _id: version,
            name,
            messages,
            token_usage,
        })
    }

    fn serialize_session(
        &self,
        path: &Path,
        history: &ChatRequest,
        token_usage: &TokenUsageLedger,
    ) -> Result<(), Box<dyn Error>> {
        let file = File::create(path)?;
        let mut ser = Serializer::new(file).with_struct_map();

        3.serialize(&mut ser)?;
        let name = self.name.clone().unwrap_or(Self::derived_name(history));
        name.serialize(&mut ser)?;
        history.messages.serialize(&mut ser)?;
        token_usage.serialize(&mut ser)?;

        Ok(())
    }

    pub fn load(&self) -> Result<(ChatRequest, TokenUsageLedger), Box<dyn Error>> {
        let Some(session_id) = self.id.clone() else {
            return Ok((ChatRequest::new(vec![]), TokenUsageLedger::default()));
        };

        let path = Dirs::session_file(session_id.as_str())?;
        if path.is_file() {
            let data = Self::deserialize_session(path.as_path(), true)?;
            Ok((
                ChatRequest::new(data.messages.unwrap_or_default()),
                data.token_usage.unwrap_or_default(),
            ))
        } else {
            Ok((ChatRequest::new(vec![]), TokenUsageLedger::default()))
        }
    }

    pub fn save(
        &self,
        history: &ChatRequest,
        token_usage: &TokenUsageLedger,
    ) -> Result<(), Box<dyn Error>> {
        if let Some(session_id) = &self.id {
            let path = Dirs::session_file(session_id.as_str())?;
            self.serialize_session(path.as_path(), history, token_usage)?;
        }

        Ok(())
    }

    pub fn list() -> Vec<Session> {
        let dir = match Dirs::sessions_dir() {
            Ok(d) => d,
            Err(_) => return Vec::new(),
        };

        let mut sessions: Vec<Session> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .filter_map(|entry| {
                        let path = entry.path();
                        if path.extension()? == "mpk" {
                            let data = Self::deserialize_session(path.as_path(), false).ok()?;
                            let id = path.file_stem()?.to_str().map(|s| s.to_string());

                            Some(Session {
                                id,
                                name: Some(data.name),
                            })
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        sessions.sort_by(|a, b| b.id.cmp(&a.id));
        sessions
    }

    pub fn delete(session_id: &str) -> Result<(), Box<dyn Error>> {
        Dirs::validate_session_id(session_id)?;
        let path = Dirs::session_file(session_id)?;
        if path.is_file() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    pub fn name(session_id: &str) -> Result<String, Box<dyn Error>> {
        let path = Dirs::session_file(session_id)?;
        if path.is_file() {
            let data = Self::deserialize_session(path.as_path(), false)?;
            Ok(data.name)
        } else {
            Err("Session file not found".into())
        }
    }

    pub fn rename(&mut self, new_name: String) -> Result<(), Box<dyn Error>> {
        let Some(session_id) = &self.id else {
            return Err("Cannot rename a session without an id".into());
        };

        let (history, token_usage) = self.load()?;
        let path = Dirs::session_file(session_id.as_str())?;
        let tmp_path = path.with_extension("tmp");

        self.name = Some(new_name);
        self.serialize_session(tmp_path.as_path(), &history, &token_usage)?;
        std::fs::rename(&tmp_path, &path)?;

        Ok(())
    }

    pub fn display_name(&self) -> String {
        self.name
            .as_deref()
            .or(self.id.as_deref())
            .unwrap_or("<none>")
            .to_string()
    }

    fn derived_name(history: &ChatRequest) -> String {
        fn first_text(content: &MessageContent) -> Option<String> {
            content.first_text().map(|s| s.to_string())
        }

        history
            .messages
            .iter()
            .find(|m| m.role == ChatRole::User)
            .and_then(|m| first_text(&m.content))
            .map(|s| {
                s.chars()
                    .take(40)
                    .filter(|c| !c.is_control())
                    .collect::<String>()
            })
            .unwrap_or("<unamed>".to_string())
    }
}

#[derive(Deserialize)]
struct TokenUsageLedgerV2 {
    spans: Vec<TokenUsageSpan>,
    last_prompt_tokens: Option<i32>,
    last_prompt_index: Option<usize>,
    last_total_tokens: Option<i32>,
}

impl From<TokenUsageLedgerV2> for TokenUsageLedger {
    fn from(v2: TokenUsageLedgerV2) -> Self {
        TokenUsageLedger {
            spans: v2.spans,
            last_prompt_tokens: v2.last_prompt_tokens,
            last_prompt_index: v2.last_prompt_index,
            usage: UsageInfo {
                context_tokens: v2.last_total_tokens,
                session_input: v2.last_prompt_tokens,
                ..Default::default()
            },
        }
    }
}
