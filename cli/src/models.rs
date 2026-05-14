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
    config::{Config, ProviderConfig},
    harness::create_client,
};
use genai::{Client, adapter::AdapterKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, error::Error, path::PathBuf, time::Duration};
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AvailableModel {
    pub provider: String,
    pub id: String,
}

impl AvailableModel {
    pub fn label(&self) -> String {
        format!("{} [{}]", self.id, self.provider)
    }

    pub fn create_client(&self, config: &Config) -> Result<Client, Box<dyn Error>> {
        let provider = config
            .providers()
            .iter()
            .find(|provider| provider.name == self.provider)
            .ok_or_else(|| format!("Model references missing provider '{}'.", self.provider))?;

        let adapter_kind =
            AdapterKind::from_lower_str(&provider.kind.to_lowercase()).ok_or_else(|| {
                format!(
                    "Provider '{}' has invalid kind '{}'.",
                    provider.name, provider.kind
                )
            })?;

        Ok(create_client(
            adapter_kind,
            provider.url.clone(),
            provider.key.clone(),
            self.id.clone(),
        ))
    }
}

type ProviderModelsCache = BTreeMap<String, Vec<String>>;

pub struct Models;

impl Models {
    pub async fn list(config: &Config) -> Result<Vec<AvailableModel>, Box<dyn Error>> {
        if let Some(models) = Self::load_cache()? {
            return Ok(models);
        }

        Self::reload(config).await
    }

    pub async fn reload(config: &Config) -> Result<Vec<AvailableModel>, Box<dyn Error>> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()?;

        let mut models = Vec::new();
        let mut errors = Vec::new();

        for provider in config.providers() {
            match Self::fetch_provider_models(&client, provider).await {
                Ok(provider_models) => models.extend(provider_models),
                Err(err) => errors.push(format!("{}: {}", provider.name, err)),
            }
        }

        models.sort_by(|a, b| a.provider.cmp(&b.provider).then_with(|| a.id.cmp(&b.id)));

        if models.is_empty() && !errors.is_empty() {
            return Err(format!("No models found. {}", errors.join("; ")).into());
        }

        Self::save_cache(&models)?;
        Ok(models)
    }

    fn load_cache() -> Result<Option<Vec<AvailableModel>>, Box<dyn Error>> {
        let path = Self::cache_path()?;
        if !path.is_file() {
            return Ok(None);
        }

        let contents = std::fs::read_to_string(path)?;
        let cache: ProviderModelsCache = serde_json::from_str(&contents)?;
        Ok(Some(models_from_cache(cache)))
    }

    fn save_cache(models: &[AvailableModel]) -> Result<(), Box<dyn Error>> {
        let path = Self::cache_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let cache = cache_from_models(models);
        std::fs::write(path, serde_json::to_string_pretty(&cache)?)?;
        Ok(())
    }

    fn cache_path() -> Result<PathBuf, Box<dyn Error>> {
        let home = std::env::var("HOME")?;
        Ok(PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("gaius")
            .join("models_cache.json"))
    }

    async fn fetch_provider_models(
        client: &reqwest::Client,
        provider: &ProviderConfig,
    ) -> Result<Vec<AvailableModel>, Box<dyn Error>> {
        let urls = list_models_urls(&provider.url)?;
        let mut last_error: Option<Box<dyn Error>> = None;

        for url in urls {
            match fetch_models_url(client, provider, url).await {
                Ok(models) => return Ok(models),
                Err(err) => last_error = Some(err),
            }
        }

        Err(last_error.unwrap_or_else(|| "No model URLs generated".into()))
    }
}

fn cache_from_models(models: &[AvailableModel]) -> ProviderModelsCache {
    let mut cache = ProviderModelsCache::new();
    for model in models {
        cache
            .entry(model.provider.clone())
            .or_default()
            .push(model.id.clone());
    }

    for model_ids in cache.values_mut() {
        model_ids.sort();
    }

    cache
}

fn models_from_cache(cache: ProviderModelsCache) -> Vec<AvailableModel> {
    cache
        .into_iter()
        .flat_map(|(provider, mut model_ids)| {
            model_ids.sort();
            model_ids.into_iter().map(move |id| AvailableModel {
                provider: provider.clone(),
                id,
            })
        })
        .collect()
}

async fn fetch_models_url(
    client: &reqwest::Client,
    provider: &ProviderConfig,
    url: Url,
) -> Result<Vec<AvailableModel>, Box<dyn Error>> {
    let request = client.get(url);
    let request = if provider.kind.eq_ignore_ascii_case("anthropic") {
        request
            .header("x-api-key", &provider.key)
            .header("anthropic-version", "2023-06-01")
    } else {
        request.bearer_auth(&provider.key)
    };

    let value = request
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;

    let model_ids = extract_model_ids(&value);
    if model_ids.is_empty() {
        return Err("Model response contained no models".into());
    }

    Ok(model_ids
        .into_iter()
        .map(|id| AvailableModel {
            provider: provider.name.clone(),
            id,
        })
        .collect())
}

fn extract_model_ids(value: &Value) -> Vec<String> {
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
                        Some(id.to_string())
                    } else {
                        item.get("id")
                            .or_else(|| item.get("name"))
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn list_models_urls(provider_url: &str) -> Result<Vec<Url>, Box<dyn Error>> {
    let mut base = Url::parse(provider_url)?;
    let mut urls = Vec::new();

    for _ in 0..2 {
        urls.push(url_with_models_path(&base)?);

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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{AvailableModel, cache_from_models, extract_model_ids, list_models_urls};

    #[test]
    fn model_urls_walks_provider_path_upward() {
        let urls = list_models_urls("https://example.com/api/v1").unwrap();
        let urls: Vec<String> = urls.into_iter().map(|url| url.to_string()).collect();

        assert_eq!(
            urls,
            vec![
                "https://example.com/api/v1/models",
                "https://example.com/api/models",
            ]
        );
    }

    #[test]
    fn model_urls_handles_trailing_slash() {
        let urls = list_models_urls("https://example.com/v1/").unwrap();
        let urls: Vec<String> = urls.into_iter().map(|url| url.to_string()).collect();

        assert_eq!(
            urls,
            vec![
                "https://example.com/v1/models",
                "https://example.com/models"
            ]
        );
    }

    #[test]
    fn extracts_openai_compatible_models() {
        let ids = extract_model_ids(&json!({
            "data": [
                { "id": "model-a" },
                { "id": "model-b" }
            ]
        }));

        assert_eq!(ids, vec!["model-a", "model-b"]);
    }

    #[test]
    fn extracts_model_arrays() {
        let ids = extract_model_ids(&json!({
            "models": [
                { "name": "model-a" },
                "model-b"
            ]
        }));

        assert_eq!(ids, vec!["model-a", "model-b"]);
    }

    #[test]
    fn model_label_puts_provider_in_brackets() {
        let model = super::AvailableModel {
            provider: "local".to_string(),
            id: "llama".to_string(),
        };

        assert_eq!(model.label(), "llama [local]");
    }

    #[test]
    fn models_cache_serializes_as_provider_model_map() {
        let cache = cache_from_models(&[
            AvailableModel {
                provider: "provider 2".to_string(),
                id: "model 4".to_string(),
            },
            AvailableModel {
                provider: "provider 1".to_string(),
                id: "model 2".to_string(),
            },
            AvailableModel {
                provider: "provider 1".to_string(),
                id: "model 1".to_string(),
            },
            AvailableModel {
                provider: "provider 2".to_string(),
                id: "model 3".to_string(),
            },
        ]);

        assert_eq!(
            serde_json::to_value(cache).unwrap(),
            json!({
                "provider 1": ["model 1", "model 2"],
                "provider 2": ["model 3", "model 4"]
            })
        );
    }

    #[test]
    fn models_cache_loads_provider_model_map() {
        let cache = serde_json::from_value(json!({
            "provider 1": ["model 2", "model 1"],
            "provider 2": ["model 3"]
        }))
        .unwrap();

        let models = super::models_from_cache(cache);
        let models: Vec<(String, String)> = models
            .into_iter()
            .map(|model| (model.provider, model.id))
            .collect();

        assert_eq!(
            models,
            vec![
                ("provider 1".to_string(), "model 1".to_string()),
                ("provider 1".to_string(), "model 2".to_string()),
                ("provider 2".to_string(), "model 3".to_string())
            ]
        );
    }
}
