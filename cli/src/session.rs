use std::{error::Error, path::PathBuf};

use genai::chat::ChatRequest;
use uuid::Uuid;

fn session_path(session_id: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    validate_session_id(session_id)?;
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("gaius")
        .join("sessions")
        .join(format!("{}.mpk", session_id)))
}

fn validate_session_id(session_id: &str) -> Result<(), Box<dyn std::error::Error>> {
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

    pub fn new_named(name: String) -> Result<Self, Box<dyn std::error::Error>> {
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
            let content = std::fs::read(&path)?;
            Ok(rmp_serde::from_slice(&content)?)
        } else {
            Ok(ChatRequest::new(vec![]))
        }
    }

    pub fn save(&self, history: &ChatRequest) -> Result<(), Box<dyn Error>> {
        if let Some(session_id) = self.id.clone() {
            let path = session_path(session_id.as_str())?;
            let content = rmp_serde::to_vec(history)?;
            std::fs::write(&path, content)?;
        }

        Ok(())
    }
}
