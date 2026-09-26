/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::{AgentDefinition, Agents},
    cli_prompt::CliPrompt,
    client::LLMClient,
    dirs::Dirs,
    models::{ModelDef, RecentModelDef},
    providers::ProviderDef,
};
use futures::StreamExt;
use genai::{
    adapter::AdapterKind,
    chat::{ChatRequest, ChatStreamEvent},
};
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
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
            "Config file missing: {}\nConfigure an LLM provider:\n",
            path.display()
        );
        loop {
            let mut kind = CliPrompt::get_input(
                "Kind (blank = OpenAI compatible; codex or grok = Codex/Grok subscriptions): ",
            )?;
            kind = if kind.is_empty() {
                "openai".to_string()
            } else {
                kind
            };

            let is_codex = kind.eq_ignore_ascii_case("codex");
            let is_grok = kind.eq_ignore_ascii_case("grok");
            if !is_codex && !is_grok && AdapterKind::from_lower_str(&kind).is_none() {
                eprintln!("Invalid provider kind: {}", kind);
                continue;
            }

            let (name, url, key, model_id) = if is_codex || is_grok {
                let model_id = CliPrompt::get_input("Model: ")?;
                let name = if is_codex { "Codex" } else { "Grok" };
                (name.to_string(), String::new(), String::new(), model_id)
            } else {
                let url = CliPrompt::get_input("Url: ")?;
                let key = CliPrompt::get_input("Key: ")?;
                let model_id = CliPrompt::get_input("Model: ")?;
                let name =
                    match Url::parse(&url).map(|u| u.host_str().unwrap_or("default").to_string()) {
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
            let provider_def = match ProviderDef::new(&provider_config) {
                Ok(provider_def) => provider_def,
                Err(err) => {
                    eprintln!("Error: {}", err);
                    continue;
                }
            };

            let model_def = ModelDef {
                provider: provider_def,
                id: model_id,
                context_len: None,
                pricing: None,
                reasoning: None,
            };

            let mut client = LLMClient::new(AgentDefinition::default());
            if let Err(err) = client.set_model(model_def.clone()).await {
                eprintln!("Error setting model: {}", err);
                continue;
            }

            match validate_model(&client).await {
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

                    // add to recent list so initial load has a model
                    RecentModelDef::add(&model_def)?;

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
        provider.validate(self)?;
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

    pub fn agents(&self) -> &Agents {
        &self.agents
    }
}

impl ProviderConfig {
    pub fn validate(&self, config: &Config) -> Result<(), Box<dyn Error>> {
        if self.name.trim().is_empty() {
            return Err("Provider name cannot be empty".into());
        }
        if config.provider.iter().any(|p| p.name == self.name) {
            return Err(format!("Provider '{}' already exists", self.name).into());
        }
        match self.kind.as_str() {
            "codex" | "grok" => {}
            _ => {
                let Some(kind) = AdapterKind::from_lower_str(&self.kind.to_lowercase()) else {
                    return Err(format!("Invalid provider kind: {}", self.kind).into());
                };
                Url::parse(&self.url)?;
                match kind {
                    AdapterKind::Ollama => {}
                    _ => {
                        if self.key.trim().is_empty() {
                            return Err("Provider key cannot be empty".into());
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

async fn validate_model(client: &LLMClient) -> Result<(), Box<dyn Error>> {
    let request = ChatRequest::from_user("Reply with OK.");
    let mut response = client.chat_streaming(request).await?;

    let mut stream_end = None;
    while let Some(event) = response.stream.next().await {
        match event {
            Ok(event) => {
                if let ChatStreamEvent::End(end) = event {
                    stream_end = Some(end);
                }
            }
            Err(err) => return Err(err.into()),
        }
    }

    stream_end.ok_or("Chat stream ended without an end event")?;
    Ok(())
}
