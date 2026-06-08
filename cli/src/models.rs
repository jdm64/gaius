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
    config::{Config, ConfiguredModel, ProviderConfig},
    util::cache_dir,
};
use genai::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, error::Error, path::PathBuf, time::Duration};
use url::Url;

pub const RECENT_MODELS_LIMIT: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDef {
    pub provider: String,
    pub id: String,
    pub context_len: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedModelDef {
    pub id: String,
    pub context_len: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentModelDef {
    pub provider: String,
    pub id: String,
}

type ProviderModelsCache = BTreeMap<String, Vec<CachedModelDef>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelPickerRow {
    Header(String),
    Separator,
    Model(ModelDef),
    RecentModel(ModelDef),
}

impl ModelDef {
    pub fn label(&self) -> String {
        format!("{} [{}]", self.id, self.provider)
    }

    pub fn similar(&self, other: &ModelDef) -> bool {
        self.provider == other.provider && self.id == other.id
    }

    pub fn create_client(&self, config: &Config) -> Result<Client, Box<dyn Error>> {
        let provider = config
            .providers()
            .iter()
            .find(|provider| provider.name == self.provider)
            .ok_or_else(|| format!("Model references missing provider '{}'.", self.provider))?;

        let selected_model = ConfiguredModel {
            provider_name: provider.name.clone(),
            provider_kind: provider.kind.clone(),
            provider_url: provider.url.clone(),
            provider_key: provider.key.clone(),
            model_id: self.id.clone(),
        };

        selected_model.create_client()
    }
}

impl CachedModelDef {
    fn load() -> Result<Option<Vec<ModelDef>>, Box<dyn Error>> {
        let path = Self::path()?;
        if !path.is_file() {
            return Ok(None);
        }

        let contents = std::fs::read_to_string(path)?;
        let cache: ProviderModelsCache = serde_json::from_str(&contents)?;
        Ok(Some(Self::to_models(cache)))
    }

    fn save(models: &[ModelDef]) -> Result<(), Box<dyn Error>> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let cache = Self::to_cache(models);
        std::fs::write(path, serde_json::to_string_pretty(&cache)?)?;
        Ok(())
    }

    fn path() -> Result<PathBuf, Box<dyn Error>> {
        Ok(cache_dir()?.join("models_cache.json"))
    }

    pub fn to_cache(models: &[ModelDef]) -> ProviderModelsCache {
        let mut cache = ProviderModelsCache::new();
        for model in models {
            cache
                .entry(model.provider.clone())
                .or_default()
                .push(CachedModelDef {
                    id: model.id.clone(),
                    context_len: model.context_len,
                });
        }

        for model_ids in cache.values_mut() {
            model_ids.sort_by(|a, b| a.id.cmp(&b.id));
        }

        cache
    }

    pub fn to_models(cache: ProviderModelsCache) -> Vec<ModelDef> {
        cache
            .into_iter()
            .flat_map(|(provider, mut cached_models)| {
                cached_models.sort_by(|a, b| a.id.cmp(&b.id));
                cached_models.into_iter().map(move |cached| ModelDef {
                    provider: provider.clone(),
                    id: cached.id,
                    context_len: cached.context_len,
                })
            })
            .collect()
    }
}

impl RecentModelDef {
    fn load_recent() -> Result<Vec<RecentModelDef>, Box<dyn Error>> {
        let path = Self::path()?;
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
            .map(|model| ((model.provider.clone(), model.id.clone()), model))
            .collect();

        recent
            .iter()
            .map(|recent| {
                cache_by_key
                    .get(&(recent.provider.clone(), recent.id.clone()))
                    .copied()
                    .cloned()
                    .unwrap_or_else(|| ModelDef {
                        provider: recent.provider.clone(),
                        id: recent.id.clone(),
                        context_len: None,
                    })
            })
            .collect()
    }

    pub fn save(recent: &[RecentModelDef]) -> Result<(), Box<dyn Error>> {
        let path = Self::path()?;
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
                provider: model.provider.clone(),
                id: model.id.clone(),
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

    fn path() -> Result<PathBuf, Box<dyn Error>> {
        Ok(cache_dir()?.join("recent_models.json"))
    }

    pub fn join(recent: &[RecentModelDef], model: &RecentModelDef) -> Vec<RecentModelDef> {
        let mut models = Vec::with_capacity(RECENT_MODELS_LIMIT);
        models.push(model.clone());

        for recent_model in recent {
            if recent_model != model && models.len() < RECENT_MODELS_LIMIT {
                models.push(recent_model.clone());
            }
        }

        models
    }

    fn same(&self, model: &ModelDef) -> bool {
        self.provider == model.provider && self.id == model.id
    }
}

pub struct Models;

impl Models {
    pub async fn list(config: &Config) -> Result<Vec<ModelDef>, Box<dyn Error>> {
        if let Some(models) = CachedModelDef::load()? {
            return Ok(models);
        }

        Self::reload(config).await
    }

    pub async fn reload(config: &Config) -> Result<Vec<ModelDef>, Box<dyn Error>> {
        let mut models = Vec::new();
        let mut errors = Vec::new();

        for provider in config.providers() {
            match provider.list_models().await {
                Ok(provider_models) => models.extend(provider_models),
                Err(err) => errors.push(format!("{}: {}", provider.name, err)),
            }
        }

        models.sort_by(|a, b| a.provider.cmp(&b.provider).then_with(|| a.id.cmp(&b.id)));

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

impl ProviderConfig {
    pub async fn list_models(&self) -> Result<Vec<ModelDef>, Box<dyn Error>> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()?;

        let urls = self.models_list_urls()?;
        let mut last_error: Option<Box<dyn Error>> = None;

        for url in urls {
            match self.fetch_models(&client, url).await {
                Ok(models) => return Ok(models),
                Err(err) => last_error = Some(err),
            }
        }

        Err(last_error.unwrap_or_else(|| "No model URLs generated".into()))
    }

    pub fn models_list_urls(&self) -> Result<Vec<Url>, Box<dyn Error>> {
        let mut base = Url::parse(self.url.as_str())?;
        let mut urls = Vec::new();

        for _ in 0..2 {
            urls.push(Self::url_with_models_path(&base)?);

            let mut segments: Vec<String> = base
                .path_segments()
                .map(|segments| segments.map(ToString::to_string).collect())
                .unwrap_or_default();
            segments.retain(|segment| !segment.is_empty());

            if segments.is_empty() {
                break;
            }

            segments.pop();
            {
                let mut path_segments = base
                    .path_segments_mut()
                    .map_err(|_| "Provider URL cannot be a base for model discovery")?;
                path_segments.clear();
                for segment in &segments {
                    path_segments.push(segment);
                }
            }
        }

        Ok(urls)
    }

    fn url_with_models_path(base: &Url) -> Result<Url, Box<dyn Error>> {
        let mut url = base.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| "Provider URL cannot be a base for model discovery")?;
            segments.pop_if_empty();
            segments.push("models");
        }
        Ok(url)
    }

    async fn fetch_models(
        &self,
        client: &reqwest::Client,
        url: Url,
    ) -> Result<Vec<ModelDef>, Box<dyn Error>> {
        let request = client.get(url);
        let request = if self.kind.eq_ignore_ascii_case("anthropic") {
            request
                .header("x-api-key", &self.key)
                .header("anthropic-version", "2023-06-01")
        } else {
            request.bearer_auth(&self.key)
        };

        let value = request
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;

        let model_defs = self.extract_model_defs(&value);
        if model_defs.is_empty() {
            return Err("Model response contained no models".into());
        }

        Ok(model_defs)
    }

    pub fn extract_model_defs(&self, value: &Value) -> Vec<ModelDef> {
        value
            .get("data")
            .or_else(|| value.get("models"))
            .or_else(|| value.as_array().map(|_| value))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        if let Some(id) = item.as_str() {
                            Some(ModelDef {
                                provider: self.name.clone(),
                                id: id.to_string(),
                                context_len: None,
                            })
                        } else {
                            let id = item
                                .get("id")
                                .or_else(|| item.get("name"))
                                .and_then(Value::as_str)
                                .map(ToString::to_string)?;

                            let context_len = item
                                .get("context_length")
                                .and_then(Value::as_i64)
                                .map(|n| n as i32);

                            Some(ModelDef {
                                provider: self.name.clone(),
                                id,
                                context_len,
                            })
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}
