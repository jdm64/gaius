/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use std::{error::Error, path::PathBuf};

pub struct Dirs;

impl Dirs {
    pub fn config_dir() -> Result<PathBuf, Box<dyn Error>> {
        let home = std::env::var("HOME")?;
        let dir = PathBuf::from(home).join(".config").join("gaius");
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    pub fn session_file(session_id: &str) -> Result<PathBuf, Box<dyn Error>> {
        Self::validate_session_id(session_id)?;
        let sessions_dir = Dirs::sessions_dir()?;
        std::fs::create_dir_all(&sessions_dir)?;
        Ok(sessions_dir.join(format!("{}.mpk", session_id)))
    }

    pub fn validate_session_id(session_id: &str) -> Result<(), Box<dyn Error>> {
        if session_id.is_empty() {
            return Err("Session id cannot be empty".into());
        }

        if session_id.contains('/') || session_id.contains('\\') {
            return Err("Session id cannot contain path separators".into());
        }

        Ok(())
    }

    pub fn config_file() -> Result<PathBuf, Box<dyn Error>> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    pub fn models_cache() -> Result<PathBuf, Box<dyn Error>> {
        Ok(Self::cache_dir()?.join("models_cache.json"))
    }

    pub fn sessions_dir() -> Result<PathBuf, Box<dyn Error>> {
        Ok(Self::data_dir()?.join("sessions"))
    }

    pub fn prompt_history_file() -> Result<PathBuf, Box<dyn Error>> {
        Ok(Self::cache_dir()?.join("prompt_history.json"))
    }

    pub fn data_dir() -> Result<PathBuf, Box<dyn Error>> {
        let home = std::env::var("HOME")?;
        let dir = PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("gaius");
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    pub fn cache_dir() -> Result<PathBuf, Box<dyn Error>> {
        let home = std::env::var("HOME")?;
        let dir = PathBuf::from(home).join(".cache").join("gaius");
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}
