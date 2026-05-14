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

use crate::{
    agents::Agents,
    harness::{create_client, validate_model},
    util::{config_dir, prompt_input},
};
use genai::Client;
use genai::adapter::AdapterKind;
use serde::{Deserialize, Serialize};
use std::{error::Error, path::PathBuf};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    provider: Vec<ProviderConfig>,
    #[serde(default)]
    model: Vec<ModelConfig>,
    #[serde(skip)]
    agents: Agents,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub kind: String,
    pub url: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    pub provider: String,
    pub id: String,
}

pub struct SelectedModel {
    pub client: Client,
    pub model_id: String,
}

pub fn config_file() -> Result<PathBuf, Box<dyn Error>> {
    Ok(config_dir()?.join("config.toml"))
}

impl Config {
    pub fn new() -> Config {
        Self {
            provider: vec![],
            model: vec![],
            agents: Agents::default(),
        }
    }

    pub async fn load(&mut self) -> Result<(), Box<dyn Error>> {
        let path = config_file()?;
        if path.exists() {
            let contents = std::fs::read_to_string(&path)?;
            *self = toml::from_str(&contents)?;
            self.agents = Agents::load(&config_dir()?)?;
            return Ok(());
        }

        println!(
            "Config file missing ({}). Configure an LLM provider.",
            path.display()
        );
        loop {
            let mut kind = prompt_input("Kind (blank for OpenAI compatable): ")?;
            kind = if kind.is_empty() {
                "openai".to_string()
            } else {
                kind
            };

            let adapter_kind = match AdapterKind::from_lower_str(&kind) {
                Some(kind) => kind,
                None => {
                    eprintln!("Invalid provider kind: {}", kind);
                    continue;
                }
            };

            let url = prompt_input("Url: ")?;
            let key = prompt_input("Key: ")?;
            let model_id = prompt_input("Model: ")?;

            let name = match Url::parse(&url).map(|u| u.host_str().unwrap_or("default").to_string())
            {
                Ok(name) => name,
                Err(_) => "default".to_string(),
            };

            let client = create_client(adapter_kind, url.clone(), key.clone(), model_id.clone());
            match validate_model(&client, &model_id).await {
                Ok(()) => {
                    let provider = ProviderConfig {
                        name: name.to_string(),
                        url,
                        kind: adapter_kind.as_lower_str().to_string(),
                        key,
                    };
                    let model = ModelConfig {
                        name: model_id.clone(),
                        provider: name.to_string(),
                        id: model_id,
                    };
                    let config = Config {
                        provider: vec![provider],
                        model: vec![model],
                        agents: Agents::load(&config_dir()?)?,
                    };
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&path, toml::to_string_pretty(&config)?)?;
                    *self = config;
                    return Ok(());
                }
                Err(err) => {
                    eprintln!("Provider validation failed: {}", err);
                }
            }
        }
    }

    pub async fn select_model(&self) -> Result<SelectedModel, Box<dyn Error>> {
        for model in &self.model {
            let provider = match self
                .provider
                .iter()
                .find(|provider| provider.name == model.provider)
            {
                Some(provider) => provider,
                None => {
                    eprintln!(
                        "Model '{}' references missing provider '{}'.",
                        model.name, model.provider
                    );
                    continue;
                }
            };

            let adapter_kind = match AdapterKind::from_lower_str(&provider.kind.to_lowercase()) {
                Some(kind) => kind,
                None => {
                    eprintln!(
                        "Provider '{}' has invalid kind '{}'.",
                        provider.name, provider.kind
                    );
                    continue;
                }
            };

            let client = create_client(
                adapter_kind,
                provider.url.clone(),
                provider.key.clone(),
                model.id.clone(),
            );

            return Ok(SelectedModel {
                client,
                model_id: model.id.clone(),
            });
        }

        Err("No valid model was found. Check config file.".into())
    }

    pub fn providers(&self) -> &[ProviderConfig] {
        &self.provider
    }

    pub fn agents(&self) -> &Agents {
        &self.agents
    }
}
