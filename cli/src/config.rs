/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::Agents,
    auth::{handle::OAuthHandle, spec::OAuthKind},
    cli_prompt::CliPrompt,
    dirs::Dirs,
    providers::ProviderDef,
};
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

        self.setup(path).await
    }

    async fn setup(&mut self, path: PathBuf) -> Result<(), Box<dyn Error>> {
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

            let sub = OAuthKind::from_lower_str(&kind);
            if sub.is_none() && AdapterKind::from_lower_str(&kind).is_none() {
                eprintln!("Invalid provider kind: {}", kind);
                continue;
            }

            let (name, url, key) = if let Some(sub) = sub {
                let spec = sub.spec();
                match OAuthHandle::get(sub).map(|auth| auth.is_logged_in()) {
                    Ok(true) => {}
                    Ok(false) => eprintln!(
                        "Not logged in to {} yet. Run 'gaius --login {}' first.",
                        spec.display, spec.id
                    ),
                    Err(err) => eprintln!(
                        "Could not read the saved {} login ({err}). \
                         Run 'gaius --login {}' to sign in again.",
                        spec.display, spec.id
                    ),
                }

                (spec.display.to_string(), String::new(), String::new())
            } else {
                let url = CliPrompt::get_input("Url: ")?;
                let key = CliPrompt::get_input("Key: ")?;
                let name =
                    match Url::parse(&url).map(|u| u.host_str().unwrap_or("default").to_string()) {
                        Ok(name) => name,
                        Err(_) => "default".to_string(),
                    };

                (name, url, key)
            };

            let provider_config = ProviderConfig {
                name,
                kind,
                url,
                key,
            };
            if let Err(err) = ProviderDef::new(&provider_config) {
                eprintln!("Error: {}", err);
                continue;
            }

            let config = Config {
                provider: vec![provider_config],
                model: vec![],
                agents: Agents::load(&Dirs::config_dir()?)?,
            };
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, toml::to_string_pretty(&config)?)?;
            *self = config;
            return Ok(());
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
        if OAuthKind::from_lower_str(&self.kind).is_some() {
            return Ok(());
        }

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

        Ok(())
    }
}
