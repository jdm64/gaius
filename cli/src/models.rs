/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{config::Config, dirs::Dirs, providers::ProviderDef};
use genai::Client;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, error::Error};

pub const RECENT_MODELS_LIMIT: usize = 8;

/// Reasoning effort level for models that support it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Default,
    None,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl ReasoningEffort {
    pub fn all() -> &'static [ReasoningEffort] {
        &[
            ReasoningEffort::Default,
            ReasoningEffort::None,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::XHigh,
            ReasoningEffort::Max,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            ReasoningEffort::Default => "default",
            ReasoningEffort::None => "none",
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::XHigh => "xhigh",
            ReasoningEffort::Max => "max",
        }
    }

    /// Convert to the genai `ReasoningEffort` variant, returning `None` for
    /// `Default` (which means "use the provider's default").
    pub fn to_genai(&self) -> Option<genai::chat::ReasoningEffort> {
        match self {
            ReasoningEffort::Default => None,
            ReasoningEffort::None => Some(genai::chat::ReasoningEffort::None),
            ReasoningEffort::Low => Some(genai::chat::ReasoningEffort::Low),
            ReasoningEffort::Medium => Some(genai::chat::ReasoningEffort::Medium),
            ReasoningEffort::High => Some(genai::chat::ReasoningEffort::High),
            ReasoningEffort::XHigh => Some(genai::chat::ReasoningEffort::XHigh),
            ReasoningEffort::Max => Some(genai::chat::ReasoningEffort::Max),
        }
    }
}

impl std::fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenPrice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_in: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_read: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_out: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ModelDef {
    pub provider: ProviderDef,
    pub id: String,
    pub context_len: Option<i32>,
    pub pricing: Option<TokenPrice>,
    pub reasoning: Option<ReasoningEffort>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CachedModelDef {
    pub id: String,
    pub context_len: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<TokenPrice>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentModelDef {
    pub provider: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningEffort>,
}

type ProviderModelsCache = BTreeMap<String, Vec<CachedModelDef>>;

#[derive(Clone, Debug, PartialEq)]
pub enum ModelPickerRow {
    Header(String),
    Separator,
    Model(ModelDef),
    RecentModel(ModelDef),
}

impl ModelDef {
    pub fn label(&self) -> String {
        format!("{} [{}]", self.id, self.provider.name())
    }

    pub fn similar(&self, other: &ModelDef) -> bool {
        self.provider.name() == other.provider.name() && self.id == other.id
    }

    pub async fn create_client(&self) -> Result<Client, Box<dyn Error>> {
        self.provider.create_client(self.id.clone()).await
    }
}

impl CachedModelDef {
    fn load(config: &Config) -> Result<Option<Vec<ModelDef>>, Box<dyn Error>> {
        let path = Dirs::models_cache()?;
        if !path.is_file() {
            return Ok(None);
        }

        let contents = std::fs::read_to_string(path)?;
        let cache: ProviderModelsCache = serde_json::from_str(&contents)?;
        Ok(Some(Self::to_models(cache, config)))
    }

    fn save(models: &[ModelDef]) -> Result<(), Box<dyn Error>> {
        let path = Dirs::models_cache()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let cache = Self::to_cache(models);
        std::fs::write(path, serde_json::to_string_pretty(&cache)?)?;
        Ok(())
    }

    pub fn to_cache(models: &[ModelDef]) -> ProviderModelsCache {
        let mut cache = ProviderModelsCache::new();
        for model in models {
            cache
                .entry(model.provider.name().to_string())
                .or_default()
                .push(CachedModelDef {
                    id: model.id.clone(),
                    context_len: model.context_len,
                    pricing: model.pricing.clone(),
                });
        }

        for model_ids in cache.values_mut() {
            model_ids.sort_by(|a, b| a.id.cmp(&b.id));
        }

        cache
    }

    pub fn to_models(cache: ProviderModelsCache, config: &Config) -> Vec<ModelDef> {
        cache
            .into_iter()
            .filter_map(|(provider_name, mut cached_models)| {
                cached_models.sort_by(|a, b| a.id.cmp(&b.id));
                let provider_def = match config
                    .providers()
                    .iter()
                    .find(|p| p.name == provider_name)
                    .map(ProviderDef::new)
                {
                    Some(Ok(provider_def)) => provider_def,
                    Some(Err(err)) => {
                        eprintln!("Skipping cached models for {provider_name}: {err}");
                        return None;
                    }
                    None => return None,
                };
                Some(cached_models.into_iter().map(move |cached| ModelDef {
                    provider: provider_def.clone(),
                    id: cached.id,
                    context_len: cached.context_len,
                    pricing: cached.pricing,
                    reasoning: None,
                }))
            })
            .flatten()
            .collect()
    }
}

impl RecentModelDef {
    fn load_recent() -> Result<Vec<RecentModelDef>, Box<dyn Error>> {
        let path = Dirs::models_recent()?;
        if !path.is_file() {
            return Ok(Vec::new());
        }

        let contents = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&contents)?)
    }

    pub fn load(cache: &[ModelDef]) -> Vec<ModelDef> {
        let recent = Self::load_recent().unwrap_or_default();
        Self::from_cache(&recent, cache)
    }

    pub fn from_cache(recent: &[RecentModelDef], cache: &[ModelDef]) -> Vec<ModelDef> {
        let cache_by_key: BTreeMap<(String, String), &ModelDef> = cache
            .iter()
            .map(|model| ((model.provider.name().to_string(), model.id.clone()), model))
            .collect();

        recent
            .iter()
            .filter_map(|recent| {
                cache_by_key
                    .get(&(recent.provider.clone(), recent.id.clone()))
                    .map(|model| {
                        let mut model = (*model).clone();
                        model.reasoning = recent.reasoning.clone();
                        model
                    })
            })
            .collect()
    }

    pub fn save(recent: &[RecentModelDef]) -> Result<(), Box<dyn Error>> {
        let path = Dirs::models_recent()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(path, serde_json::to_string_pretty(recent)?)?;
        Ok(())
    }

    pub fn add(model: &ModelDef) -> Result<Vec<RecentModelDef>, Box<dyn Error>> {
        let recent = Self::load_recent()?;
        let recent = Self::join(
            &recent,
            &RecentModelDef {
                provider: model.provider.name().to_string(),
                id: model.id.clone(),
                reasoning: model.reasoning.clone(),
            },
        );
        Self::save(&recent)?;
        Ok(recent)
    }

    pub fn remove(model: &ModelDef) -> Result<Vec<RecentModelDef>, Box<dyn Error>> {
        let recent = Self::load_recent()?;
        let recent: Vec<RecentModelDef> = recent
            .into_iter()
            .filter(|recent_model| !recent_model.same(model))
            .collect();
        Self::save(&recent)?;
        Ok(recent)
    }

    pub fn join(recent: &[RecentModelDef], model: &RecentModelDef) -> Vec<RecentModelDef> {
        let mut models = Vec::with_capacity(RECENT_MODELS_LIMIT);
        models.push(model.clone());

        for recent_model in recent {
            if !recent_model.same_model(model) && models.len() < RECENT_MODELS_LIMIT {
                models.push(recent_model.clone());
            }
        }

        models
    }

    fn same_model(&self, other: &RecentModelDef) -> bool {
        self.provider == other.provider && self.id == other.id
    }

    fn same(&self, model: &ModelDef) -> bool {
        self.provider == model.provider.name() && self.id == model.id
    }
}

pub struct Models;

impl Models {
    pub fn first_from_config(
        config: &Config,
        cache: &[ModelDef],
    ) -> Result<ModelDef, Box<dyn Error>> {
        let configured_models = config.configured_models();
        if configured_models.is_empty() {
            return Err("Unable to find configured model".into());
        }

        if cache.is_empty() {
            return Err("Unable to load model cache".into());
        }

        if let Some(found) = configured_models.iter().find_map(|selected_model| {
            cache
                .iter()
                .find(|model| {
                    model.provider.name() == selected_model.model.provider
                        && model.id == selected_model.model.id
                })
                .cloned()
        }) {
            return Ok(found);
        }

        Err(format!(
            "None of the configured models found in cached models: {}",
            configured_models
                .iter()
                .map(|model| model.model.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into())
    }

    pub fn first_from_recent(cache: &[ModelDef]) -> Option<ModelDef> {
        if cache.is_empty() {
            return None;
        }

        RecentModelDef::load(cache).into_iter().next()
    }

    pub async fn list(config: &Config) -> Result<Vec<ModelDef>, Box<dyn Error>> {
        if let Some(models) = CachedModelDef::load(config)?
            && !models.is_empty()
        {
            return Ok(models);
        }

        Self::reload(config).await
    }

    pub async fn reload(config: &Config) -> Result<Vec<ModelDef>, Box<dyn Error>> {
        let mut models = Vec::new();
        let mut errors = Vec::new();

        for provider in config.providers() {
            let provider_def = match ProviderDef::new(provider) {
                Ok(provider_def) => provider_def,
                Err(err) => {
                    errors.push(format!("{}: {}", provider.name, err));
                    continue;
                }
            };
            match provider_def.list_models().await {
                Ok(provider_models) => models.extend(provider_models),
                Err(err) => errors.push(format!("{}: {}", provider.name, err)),
            }
        }

        models.sort_by(|a, b| {
            a.provider
                .name()
                .cmp(b.provider.name())
                .then_with(|| a.id.cmp(&b.id))
        });

        if models.is_empty() && !errors.is_empty() {
            return Err(format!("No models found. {}", errors.join("; ")).into());
        }

        CachedModelDef::save(&models)?;
        Ok(models)
    }

    pub fn filter_rows(
        input: &str,
        models: &[ModelDef],
        recent: &[ModelDef],
    ) -> Vec<ModelPickerRow> {
        let recent_models = Self::filter(input, recent);
        let remaining_models: Vec<ModelDef> = Self::filter(input, models)
            .into_iter()
            .filter(|model| {
                !recent
                    .iter()
                    .any(|recent_model| recent_model.similar(model))
            })
            .collect();

        let mut rows = Vec::new();
        if !recent_models.is_empty() {
            rows.push(ModelPickerRow::Header("Recent".to_string()));
            rows.extend(recent_models.into_iter().map(ModelPickerRow::RecentModel));
        }

        if !rows.is_empty() && !remaining_models.is_empty() {
            rows.push(ModelPickerRow::Separator);
        }

        rows.extend(remaining_models.into_iter().map(ModelPickerRow::Model));
        rows
    }

    fn filter(input: &str, models: &[ModelDef]) -> Vec<ModelDef> {
        let query = input.trim().to_lowercase();
        models
            .iter()
            .filter(|model| query.is_empty() || model.id.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }
}
