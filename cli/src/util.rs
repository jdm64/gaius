/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use std::error::Error;
use std::io::{self, Write};
use std::path::PathBuf;

pub fn config_dir() -> Result<PathBuf, Box<dyn Error>> {
    let home = std::env::var("HOME")?;
    let dir = PathBuf::from(home).join(".config").join("gaius");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
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

pub fn prompt_input(label: &str) -> Result<String, Box<dyn Error>> {
    print!("{}", label);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}
