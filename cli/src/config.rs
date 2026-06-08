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
    harness::validate_model,
    util::{config_dir, prompt_input},
};
use genai::{
    Client, ModelIden, ServiceTarget,
    adapter::AdapterKind,
    resolver::{AuthData, Endpoint, ServiceTargetResolver},
};
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

pub struct ConfiguredModel {
    pub provider_name: String,
    pub provider_kind: String,
    pub provider_url: String,
    pub provider_key: String,
    pub model_id: String,
}

impl ConfiguredModel {
    pub fn new(provider: ProviderConfig, model: ModelConfig) -> ConfiguredModel {
        ConfiguredModel {
            provider_name: provider.name,
            provider_kind: provider.kind,
            provider_url: provider.url,
            provider_key: provider.key,
            model_id: model.id,
        }
    }

    pub fn create_client(&self) -> Result<Client, Box<dyn Error>> {
        let kind =
            AdapterKind::from_lower_str(&self.provider_kind.to_lowercase()).ok_or_else(|| {
                format!(
                    "Provider '{}' has invalid kind '{}'.",
                    self.provider_name, self.provider_kind
                )
            })?;

        Ok(Self::raw_create_client(
            kind,
            self.provider_url.clone(),
            self.provider_key.clone(),
            self.model_id.clone(),
        ))
    }

    fn raw_create_client(kind: AdapterKind, url: String, key: String, model: String) -> Client {
        let resolver = ServiceTargetResolver::from_resolver_fn(
            move |mut service_target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
                service_target.endpoint = Endpoint::from_owned(url.clone());
                service_target.auth = AuthData::Key(key.clone());
                service_target.model = ModelIden::new(kind, model.clone());
                Ok(service_target)
            },
        );
        Client::builder()
            .with_service_target_resolver(resolver)
            .build()
    }
}

pub fn config_file() -> Result<PathBuf, Box<dyn Error>> {
    Ok(config_dir()?.join("config.toml"))
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
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

            let model = ConfiguredModel {
                provider_name: name,
                provider_kind: adapter_kind.as_lower_str().to_string(),
                provider_url: url,
                provider_key: key,
                model_id,
            };

            let client = match model.create_client() {
                Ok(client) => client,
                Err(err) => {
                    eprintln!("Error creating client: {}", err);
                    continue;
                }
            };

            match validate_model(&client, &model.model_id.clone()).await {
                Ok(()) => {
                    let provider = ProviderConfig {
                        name: model.provider_name.clone(),
                        url: model.provider_url,
                        kind: model.provider_kind,
                        key: model.provider_key.clone(),
                    };
                    let model = ModelConfig {
                        name: model.model_id.clone(),
                        provider: model.provider_name.clone(),
                        id: model.model_id.clone(),
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

    pub fn configured_models(&self) -> Vec<ConfiguredModel> {
        self.model
            .iter()
            .filter_map(|model| {
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
                        return None;
                    }
                };

                Some(ConfiguredModel::new(provider.clone(), model.clone()))
            })
            .collect()
    }

    pub fn providers(&self) -> &[ProviderConfig] {
        &self.provider
    }

    pub fn add_provider(&mut self, provider: ProviderConfig) -> Result<(), Box<dyn Error>> {
        self.validate_provider_config(&provider)?;
        self.provider.push(provider);
        self.save()
    }

    fn save(&self) -> Result<(), Box<dyn Error>> {
        let path = config_file()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn validate_provider_config(
        &self,
        provider: &ProviderConfig,
    ) -> Result<(), Box<dyn Error>> {
        if provider.name.trim().is_empty() {
            return Err("Provider name cannot be empty".into());
        }
        if self.provider.iter().any(|p| p.name == provider.name) {
            return Err(format!("Provider '{}' already exists", provider.name).into());
        }
        if AdapterKind::from_lower_str(&provider.kind.to_lowercase()).is_none() {
            return Err(format!("Invalid provider kind: {}", provider.kind).into());
        }
        Url::parse(&provider.url)?;
        if provider.key.trim().is_empty() {
            return Err("Provider key cannot be empty".into());
        }
        Ok(())
    }

    pub fn agents(&self) -> &Agents {
        &self.agents
    }
}
