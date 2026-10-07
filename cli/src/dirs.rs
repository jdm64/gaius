/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use std::{error::Error, path::PathBuf};

pub struct Dirs;

impl Dirs {
    ///
    /// --- Config Dirs ---
    ///
    pub fn config_dir() -> Result<PathBuf, Box<dyn Error>> {
        let home = std::env::var("HOME")?;
        let dir = PathBuf::from(home).join(".config").join("gaius");
        Ok(dir)
    }

    pub fn config_file() -> Result<PathBuf, Box<dyn Error>> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    pub fn display_prefs_file() -> Result<PathBuf, Box<dyn Error>> {
        Ok(Self::cache_dir()?.join("prefs_display.json"))
    }

    ///
    /// --- Data Dirs ---
    ///
    pub fn data_dir() -> Result<PathBuf, Box<dyn Error>> {
        let home = std::env::var("HOME")?;
        let dir = PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("gaius");
        Ok(dir)
    }

    pub fn sessions_dir() -> Result<PathBuf, Box<dyn Error>> {
        Ok(Self::data_dir()?.join("sessions"))
    }

    pub fn auth_file(provider: &str) -> Result<PathBuf, Box<dyn Error>> {
        Self::validate_provider(provider)?;
        Ok(Self::data_dir()?.join(format!("auth_{provider}.json")))
    }

    pub fn session_file(session_id: &str) -> Result<PathBuf, Box<dyn Error>> {
        Self::validate_session_id(session_id)?;
        Ok(Self::sessions_dir()?.join(format!("{}.mpk", session_id)))
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

    pub fn validate_provider(provider: &str) -> Result<(), Box<dyn Error>> {
        if provider.is_empty() {
            return Err("Provider name cannot be empty".into());
        }

        if provider.contains('/') || provider.contains('\\') {
            return Err("Provider name cannot contain path separators".into());
        }

        if provider == "." || provider == ".." {
            return Err("Provider name cannot be a path component".into());
        }

        Ok(())
    }

    ///
    /// --- Cache Dirs ---
    ///
    pub fn cache_dir() -> Result<PathBuf, Box<dyn Error>> {
        let home = std::env::var("HOME")?;
        let dir = PathBuf::from(home).join(".cache").join("gaius");
        Ok(dir)
    }

    pub fn models_cache() -> Result<PathBuf, Box<dyn Error>> {
        Ok(Self::cache_dir()?.join("models_cache.json"))
    }

    pub fn models_recent() -> Result<PathBuf, Box<dyn Error>> {
        Ok(Dirs::cache_dir()?.join("models_recent.json"))
    }

    pub fn prompt_history_file() -> Result<PathBuf, Box<dyn Error>> {
        Ok(Self::cache_dir()?.join("prompt_history.json"))
    }
}
