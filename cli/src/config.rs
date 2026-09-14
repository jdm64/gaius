/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::Agents,
    cli_prompt::CliPrompt,
    dirs::Dirs,
    models::{ModelDef, ProviderDef},
};
use genai::{Client, adapter::AdapterKind, chat::ChatRequest};
use serde::{Deserialize, Serialize};
use std::error::Error;
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

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
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
    pub provider: ProviderConfig,
    pub model: ModelConfig,
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
        let path = Dirs::config_file()?;
        if path.exists() {
            let contents = std::fs::read_to_string(&path)?;
            *self = toml::from_str(&contents)?;
            self.agents = Agents::load(&Dirs::config_dir()?)?;
            return Ok(());
        }

        println!(
            "Config file missing ({}). Configure an LLM provider.",
            path.display()
        );
        loop {
            let mut kind = CliPrompt::get_input("Kind (blank for OpenAI compatable): ")?;
            kind = if kind.is_empty() {
                "openai".to_string()
            } else {
                kind
            };

            let is_oauth_openai = kind.eq_ignore_ascii_case("oauth-openai");
            if !is_oauth_openai && AdapterKind::from_lower_str(&kind).is_none() {
                eprintln!("Invalid provider kind: {}", kind);
                continue;
            }

            let (name, url, key, model_id) = if is_oauth_openai {
                let model_id = CliPrompt::get_input("Model: ")?;
                ("openai".to_string(), String::new(), String::new(), model_id)
            } else {
                let url = CliPrompt::get_input("Url: ")?;
                let key = CliPrompt::get_input("Key: ")?;
                let model_id = CliPrompt::get_input("Model: ")?;
                let name = match Url::parse(&url)
                    .map(|u| u.host_str().unwrap_or("default").to_string())
                {
                    Ok(name) => name,
                    Err(_) => "default".to_string(),
                };

                (name, url, key, model_id)
            };

            let provider_config = ProviderConfig {
                name,
                kind,
                url,
                key,
            };
            // For oauth-openai this also reads ~/.codex/auth.json, so a
            // missing/expired codex login sends us back to the prompt.
            let provider_def = match ProviderDef::new(&provider_config) {
                Ok(provider_def) => provider_def,
                Err(err) => {
                    eprintln!("Error: {}", err);
                    continue;
                }
            };

            // List models before validating so the provider's raw response is
            // dumped to disk; check the dumped ids if validation fails below.
            match provider_def.list_models().await {
                Ok(models) => {
                    println!(
                        "Found {} models; response dumped to {}",
                        models.len(),
                        Dirs::models_list_dump()?.display()
                    );
                }
                Err(err) => {
                    eprintln!(
                        "Model list request failed: {} (see {})",
                        err,
                        Dirs::models_list_dump()?.display()
                    );
                }
            }

            let model_def = ModelDef {
                provider: provider_def,
                id: model_id,
                context_len: None,
                pricing: None,
                reasoning: None,
            };

            let client = match model_def.create_client() {
                Ok(client) => client,
                Err(err) => {
                    eprintln!("Error creating client: {}", err);
                    continue;
                }
            };

            match validate_model(&client, &model_def.id.clone()).await {
                Ok(()) => {
                    let model = ModelConfig {
                        name: model_def.id.clone(),
                        provider: model_def.provider.name().to_string(),
                        id: model_def.id.clone(),
                    };
                    let config = Config {
                        provider: vec![provider_config],
                        model: vec![model],
                        agents: Agents::load(&Dirs::config_dir()?)?,
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

                Some(ConfiguredModel {
                    provider: provider.clone(),
                    model: model.clone(),
                })
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
        let path = Dirs::config_file()?;
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
        // oauth-openai is a special kind that reads tokens from ~/.codex/auth.json
        if !provider.kind.eq_ignore_ascii_case("oauth-openai") {
            if AdapterKind::from_lower_str(&provider.kind.to_lowercase()).is_none() {
                return Err(format!("Invalid provider kind: {}", provider.kind).into());
            }
            Url::parse(&provider.url)?;
            if provider.key.trim().is_empty() {
                return Err("Provider key cannot be empty".into());
            }
        }
        Ok(())
    }

    pub fn agents(&self) -> &Agents {
        &self.agents
    }
}

async fn validate_model(client: &Client, model: &str) -> Result<(), Box<dyn std::error::Error>> {
    let request = ChatRequest::from_user("Reply with ok.");
    client.exec_chat(model, request, None).await?;
    Ok(())
}
