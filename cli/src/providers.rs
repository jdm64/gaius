/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    auth::handle::OAuthHandle,
    auth::spec::OAuthKind,
    config::ProviderConfig,
    models::{ModelDef, TokenPrice},
};
use genai::{
    Client, Headers, ModelIden, ServiceTarget,
    adapter::AdapterKind,
    resolver::{AuthData, Endpoint, ServiceTargetResolver},
};

use serde_json::Value;
use std::{env, error::Error, sync::Arc, time::Duration};
use url::Url;

const CODEX_RESPONSES: &str = "https://chatgpt.com/backend-api/codex/responses";
const GROK_RESPONSES: &str = "https://cli-chat-proxy.grok.com/v1/responses";

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
        auth: Arc<OAuthHandle>,
    },
    Grok {
        name: String,
        auth: Arc<OAuthHandle>,
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

impl ProviderDef {
    pub fn new(config: &ProviderConfig) -> Result<Self, Box<dyn Error>> {
        let Some(kind) = OAuthKind::from_lower_str(&config.kind) else {
            return Ok(ProviderDef::ApiKey {
                name: config.name.clone(),
                kind: config.kind.clone(),
                url: config.url.clone(),
                key: config.key.clone(),
            });
        };

        let name = config.name.clone();
        let auth = OAuthHandle::get(kind)?;
        Ok(match kind {
            OAuthKind::Codex => ProviderDef::Codex { name, auth },
            OAuthKind::Grok => ProviderDef::Grok { name, auth },
        })
    }

    pub fn name(&self) -> &str {
        match self {
            ProviderDef::ApiKey { name, .. }
            | ProviderDef::Codex { name, .. }
            | ProviderDef::Grok { name, .. } => name,
        }
    }

    pub fn needs_refresh(&self) -> bool {
        match self {
            ProviderDef::Codex { auth, .. } | ProviderDef::Grok { auth, .. } => {
                auth.token().is_some_and(|token| token.needs_refresh())
            }
            ProviderDef::ApiKey { .. } => false,
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
            ProviderDef::Codex { auth, .. } => {
                user_agent = format!(
                    "codex_cli_rs/0.160.1 ({}; {})",
                    env::consts::OS,
                    env::consts::ARCH,
                );
                headers.merge([("originator", "codex_cli_rs".to_string())]);
                if let Some(account_id) = auth.account_id() {
                    headers.merge([("chatgpt-account-id", account_id)]);
                }
            }
            ProviderDef::Grok { .. } => {
                user_agent = format!(
                    "grok-shell/1.0.46 ({}; {})",
                    env::consts::OS,
                    env::consts::ARCH,
                );
                headers.merge([
                    ("X-XAI-Token-Auth", "xai-grok-cli".to_string()),
                    ("x-grok-client-identifier", "grok-shell".to_string()),
                    ("x-grok-client-version", "1.0.46".to_string()),
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

    pub async fn create_client(&self, model: String) -> Result<Client, Box<dyn Error>> {
        match self {
            ProviderDef::Codex { auth, .. } => Ok(Self::raw_create_client(
                AdapterKind::OpenAIResp,
                CODEX_RESPONSES.to_string(),
                auth.access_token().await?,
                model,
            )),
            ProviderDef::Grok { auth, .. } => Ok(Self::raw_create_client(
                AdapterKind::Xai,
                GROK_RESPONSES.to_string(),
                auth.access_token().await?,
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

    pub fn models_list_urls(&self) -> Result<Vec<Url>, Box<dyn Error>> {
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
            ProviderDef::Codex { auth, .. } | ProviderDef::Grok { auth, .. } => {
                request.bearer_auth(auth.access_token().await?)
            }
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
