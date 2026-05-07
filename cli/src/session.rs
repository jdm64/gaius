use std::{error::Error, fs::File, io::BufReader, path::PathBuf};

use genai::chat::{ChatMessage, ChatRequest};
use rmp_serde::{Deserializer, Serializer};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn session_path(session_id: &str) -> Result<PathBuf, Box<dyn Error>> {
    validate_session_id(session_id)?;
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("gaius")
        .join("sessions")
        .join(format!("{}.mpk", session_id)))
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

        let path = session_path(session_id.as_str())?;
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
            let path = session_path(session_id.as_str())?;
            let file = File::create(path)?;
            let mut ser = Serializer::new(file).with_struct_map();
            1.serialize(&mut ser)?;
            history.messages.serialize(&mut ser)?;
        }

        Ok(())
    }
}
