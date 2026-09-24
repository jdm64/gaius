/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    config::{Config, ProviderConfig},
    dirs::Dirs,
};
use genai::{
    Client, Headers, ModelIden, ServiceTarget,
    adapter::AdapterKind,
    resolver::{AuthData, Endpoint, ServiceTargetResolver},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, env, error::Error, path::PathBuf, time::Duration};
use url::Url;

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

#[derive(Clone, Debug, PartialEq)]
pub enum ProviderDef {
    ApiKey {
        name: String,
        kind: String,
        url: String,
        key: String,
    },
    Codex {
        name: String,
        access_token: String,
        account_id: String,
    },
    Grok {
        name: String,
        access_token: String,
    },
}

impl Default for ProviderDef {
    fn default() -> Self {
        ProviderDef::ApiKey {
            name: String::new(),
            kind: String::new(),
            url: String::new(),
            key: String::new(),
        }
    }
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

impl ProviderDef {
    pub fn new(config: &ProviderConfig) -> Result<Self, Box<dyn Error>> {
        if config.kind.eq_ignore_ascii_case("codex") {
            let (access_token, account_id) = Self::load_codex_tokens()?;
            Ok(ProviderDef::Codex {
                name: config.name.clone(),
                access_token,
                account_id,
            })
        } else if config.kind.eq_ignore_ascii_case("grok") {
            Ok(ProviderDef::Grok {
                name: config.name.clone(),
                access_token: Self::load_grok_token()?,
            })
        } else {
            Ok(ProviderDef::ApiKey {
                name: config.name.clone(),
                kind: config.kind.clone(),
                url: config.url.clone(),
                key: config.key.clone(),
            })
        }
    }

    pub fn name(&self) -> &str {
        match self {
            ProviderDef::ApiKey { name, .. }
            | ProviderDef::Codex { name, .. }
            | ProviderDef::Grok { name, .. } => name,
        }
    }

    pub fn kind_str(&self) -> &str {
        match self {
            ProviderDef::ApiKey { kind, .. } => kind,
            ProviderDef::Codex { .. } => "codex",
            ProviderDef::Grok { .. } => "grok",
        }
    }

    pub fn add_headers(&self, headers: &mut Headers) {
        let user_agent: String;
        match self {
            ProviderDef::Codex { account_id, .. } => {
                user_agent = format!(
                    "codex_cli_rs/0.155.0 ({}; {})",
                    env::consts::OS,
                    env::consts::ARCH,
                );
                headers.merge([
                    ("chatgpt-account-id", account_id.clone()),
                    ("originator", "codex_cli_rs".to_string()),
                ]);
            }
            ProviderDef::Grok { .. } => {
                user_agent = format!(
                    "grok-shell/0.2.101 ({}; {})",
                    env::consts::OS,
                    env::consts::ARCH,
                );
                headers.merge([
                    ("X-XAI-Token-Auth", "xai-grok-cli".to_string()),
                    ("x-grok-client-identifier", "grok-shell".to_string()),
                    ("x-grok-client-version", "0.2.101".to_string()),
                ]);
            }
            _ => {
                user_agent = format!(
                    "Gaius/{} ({}; {})",
                    env!("GIT_VERSION"),
                    env::consts::OS,
                    env::consts::ARCH,
                );
            }
        }
        headers.merge([("User-Agent", user_agent)]);
    }

    pub fn create_client(&self, model: String) -> Result<Client, Box<dyn Error>> {
        match self {
            ProviderDef::Codex { access_token, .. } => Ok(Self::raw_create_client(
                AdapterKind::OpenAIResp,
                "https://chatgpt.com/backend-api/codex/responses".to_string(),
                access_token.clone(),
                model,
            )),
            ProviderDef::Grok { access_token, .. } => Ok(Self::raw_create_client(
                AdapterKind::Xai,
                "https://cli-chat-proxy.grok.com/v1/responses".to_string(),
                access_token.clone(),
                model,
            )),
            ProviderDef::ApiKey { kind, url, key, .. } => {
                let kind = AdapterKind::from_lower_str(&kind.to_lowercase())
                    .ok_or_else(|| format!("Invalid provider kind '{}'.", kind))?;
                Ok(Self::raw_create_client(
                    kind,
                    url.clone(),
                    key.clone(),
                    model,
                ))
            }
        }
    }

    /// Load the credentials written by `codex login` from `~/.codex/auth.json`.
    fn load_codex_tokens() -> Result<(String, String), Box<dyn Error>> {
        let home = std::env::var("HOME")?;
        let auth_path = PathBuf::from(home).join(".codex").join("auth.json");

        if !auth_path.exists() {
            return Err(format!("OAuth auth file not found at {}", auth_path.display()).into());
        }

        let contents = std::fs::read_to_string(&auth_path)?;
        let auth_json: Value = serde_json::from_str(&contents)?;
        let tokens = auth_json
            .get("tokens")
            .ok_or("Invalid auth.json format: missing tokens")?;

        let access_token = tokens
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or("Invalid auth.json format: missing tokens.access_token")?;
        let account_id = tokens
            .get("account_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or("Invalid auth.json format: missing tokens.account_id")?;

        Ok((access_token.to_string(), account_id.to_string()))
    }

    fn load_grok_token() -> Result<String, Box<dyn Error>> {
        let home = std::env::var("HOME")?;
        let auth_path = PathBuf::from(home).join(".grok").join("auth.json");
        let contents = std::fs::read_to_string(&auth_path)
            .map_err(|err| format!("Unable to read {}: {}", auth_path.display(), err))?;
        let auth_json: Value = serde_json::from_str(&contents)?;
        let token = auth_json
            .as_object()
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|(key, _)| key.starts_with("https://auth.x.ai::"))
            })
            .and_then(|(_, value)| value.get("key"))
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or("Invalid Grok auth.json: no non-empty key for https://auth.x.ai")?;
        Ok(token.to_string())
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

    fn models_list_urls(&self) -> Result<Vec<Url>, Box<dyn Error>> {
        match self {
            ProviderDef::Codex { .. } => Ok(vec![Url::parse(
                "https://chatgpt.com/backend-api/codex/models?client_version=0.155.0",
            )?]),
            ProviderDef::Grok { .. } => Ok(vec![Url::parse(
                "https://cli-chat-proxy.grok.com/v1/models",
            )?]),
            ProviderDef::ApiKey { url, .. } => {
                let mut base = Url::parse(url.as_str())?;
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
        }
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
        let request = client.get(url.clone());
        let request = match self {
            ProviderDef::Codex { access_token, .. } => request.bearer_auth(access_token),
            ProviderDef::Grok { access_token, .. } => request.bearer_auth(access_token),
            ProviderDef::ApiKey { kind, key, .. } => {
                if kind.eq_ignore_ascii_case("anthropic") {
                    request
                        .header("x-api-key", key)
                        .header("anthropic-version", "2023-06-01")
                } else {
                    request.bearer_auth(key)
                }
            }
        };

        let response = request.send().await?;
        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            return Err(format!("GET {} returned {}", url, status).into());
        }

        let value: Value = serde_json::from_str(&body)
            .map_err(|err| format!("GET {} returned invalid JSON: {}", url, err))?;

        let model_defs = match self {
            ProviderDef::Codex { .. } => self.extract_model_defs_codex(&value),
            ProviderDef::ApiKey { .. } | ProviderDef::Grok { .. } => {
                self.extract_model_defs(&value)
            }
        };
        if model_defs.is_empty() {
            return Err("Model response contained no models".into());
        }

        Ok(model_defs)
    }

    pub fn extract_model_defs_codex(&self, value: &Value) -> Vec<ModelDef> {
        value
            .get("models")
            .or_else(|| value.get("data"))
            .or_else(|| value.as_array().map(|_| value))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let id = item
                            .get("slug")
                            .or_else(|| item.get("id"))
                            .or_else(|| item.get("name"))
                            .and_then(Value::as_str)
                            .map(ToString::to_string)?;

                        let context_len = item
                            .get("max_context_window")
                            .or_else(|| item.get("context_window"))
                            .or_else(|| item.get("context_length"))
                            .and_then(Value::as_i64)
                            .map(|n| n as i32);

                        Some(ModelDef {
                            provider: self.clone(),
                            id,
                            context_len,
                            pricing: None,
                            reasoning: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
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
                                provider: self.clone(),
                                id: id.to_string(),
                                context_len: None,
                                pricing: None,
                                reasoning: None,
                            })
                        } else {
                            let id = item
                                .get("id")
                                .or_else(|| item.get("name"))
                                .and_then(Value::as_str)
                                .map(ToString::to_string)?;

                            let context_len = item
                                .get("context_length")
                                .or_else(|| item.get("context_window"))
                                .and_then(Value::as_i64)
                                .map(|n| n as i32);

                            let pricing = item.get("pricing").and_then(|p| {
                                let price_in = p
                                    .get("prompt")
                                    .and_then(Value::as_str)
                                    .and_then(|s| s.parse::<f64>().ok());
                                let price_out = p
                                    .get("completion")
                                    .and_then(Value::as_str)
                                    .and_then(|s| s.parse::<f64>().ok());
                                let price_read = p
                                    .get("input_cache_read")
                                    .and_then(Value::as_str)
                                    .and_then(|s| s.parse::<f64>().ok());

                                if price_in.is_some() || price_out.is_some() || price_read.is_some()
                                {
                                    Some(TokenPrice {
                                        price_in,
                                        price_read,
                                        price_out,
                                    })
                                } else {
                                    None
                                }
                            });

                            Some(ModelDef {
                                provider: self.clone(),
                                id,
                                context_len,
                                pricing,
                                reasoning: None,
                            })
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl ModelDef {
    pub fn label(&self) -> String {
        format!("{} [{}]", self.id, self.provider.name())
    }

    pub fn similar(&self, other: &ModelDef) -> bool {
        self.provider.name() == other.provider.name() && self.id == other.id
    }

    pub fn create_client(&self) -> Result<Client, Box<dyn Error>> {
        self.provider.create_client(self.id.clone())
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
                let provider_def = config
                    .providers()
                    .iter()
                    .find(|p| p.name == provider_name)
                    .map(ProviderDef::new)
                    .and_then(Result::ok)?;
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
    pub async fn first_from_config(config: &Config) -> Result<ModelDef, Box<dyn Error>> {
        let configured_models = config.configured_models();
        let Some(selected_model) = configured_models.first() else {
            return Err("Unable to find configured model".into());
        };

        let cached_models = Models::list(config).await.unwrap_or_default();
        if cached_models.is_empty() {
            return Err("Unable to load model cache".into());
        }

        let found = cached_models
            .iter()
            .find(|model| {
                model.provider.name() == selected_model.model.provider
                    && model.id == selected_model.model.id
            })
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Model '{}' not found in cached models",
                    selected_model.model.id
                )
            })?;

        Ok(found)
    }

    pub async fn first_from_recent(config: &Config) -> Option<ModelDef> {
        let cached_models = Models::list(config).await.unwrap_or_default();
        if cached_models.is_empty() {
            return None;
        }

        RecentModelDef::load(&cached_models).into_iter().next()
    }

    pub async fn list(config: &Config) -> Result<Vec<ModelDef>, Box<dyn Error>> {
        if let Some(models) = CachedModelDef::load(config)? {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn api_key_provider(url: &str) -> ProviderDef {
        ProviderDef::ApiKey {
            name: String::new(),
            kind: "openai".to_string(),
            url: url.to_string(),
            key: String::new(),
        }
    }

    #[test]
    fn model_urls_walks_provider_path_upward() {
        let provider = api_key_provider("https://example.com/api/v1");
        let urls = provider.models_list_urls().unwrap();
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
        let provider = api_key_provider("https://example.com/v1/");
        let urls = provider.models_list_urls().unwrap();
        let urls: Vec<String> = urls.into_iter().map(|url| url.to_string()).collect();

        assert_eq!(
            urls,
            vec![
                "https://example.com/v1/models",
                "https://example.com/models"
            ]
        );
    }
}
