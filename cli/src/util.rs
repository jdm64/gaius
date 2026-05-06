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

use std::io::{self, Write};
use std::path::PathBuf;

pub fn config_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("gaius")
        .join("config.toml"))
}

pub fn session_path(session_id: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    validate_session_id(session_id)?;
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("gaius")
        .join("sessions")
        .join(format!("{}.mpk", session_id)))
}

pub fn validate_session_id(session_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    if session_id.is_empty() {
        return Err("Session id cannot be empty".into());
    }

    if session_id.contains('/') || session_id.contains('\\') {
        return Err("Session id cannot contain path separators".into());
    }

    Ok(())
}

pub fn prompt_input(label: &str) -> Result<String, Box<dyn std::error::Error>> {
    print!("{}", label);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}
