/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    auth::{self, OAuth, TokenError, TokenResponse, describe_error},
    dirs::Dirs,
};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, watch};
use url::Url;

pub const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_ISSUER: &str = "https://auth.openai.com";
const AUTHORIZE_PATH: &str = "/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const SCOPE: &str = "openid profile email offline_access";
const ORIGINATOR: &str = "codex_cli_rs";
pub const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
pub const LOOPBACK_ADDR: &str = "localhost:1455";
const NOT_LOGGED_IN: &str = "Not logged in to Codex. Run 'gaius --login codex' first.";
pub const AUTH_CLAIMS: &str = "https://api.openai.com/auth";
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const REFRESH_SKEW: Duration = Duration::from_mins(5);
const ASSUMED_LIFETIME: Duration = Duration::from_hours(5 * 24);

/// Codex OAuth response has these fields:
/// - access_token, token_type, expires_in, scope, id_token
/// - earliest_refresh_at, refresh_token, oai_is
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CodexToken {
    pub access_token: String,
    pub id_token: String,
    pub refresh_token: String,
    pub account_id: String,
    pub expires: i64,
}

impl CodexToken {
    pub fn new(
        response: &TokenResponse,
        previous: Option<&CodexToken>,
    ) -> Result<CodexToken, Box<dyn Error>> {
        let id_token = response
            .id_token
            .clone()
            .or_else(|| previous.map(|token| token.id_token.clone()))
            .ok_or("Codex token response did not include an id token")?;
        let refresh_token = response
            .refresh_token
            .clone()
            .or_else(|| previous.map(|token| token.refresh_token.clone()))
            .ok_or("Codex token response did not include a refresh token")?;
        let account_id = Self::account_id(&id_token)
            .or_else(|| previous.map(|token| token.account_id.clone()))
            .ok_or("Codex token response did not include a ChatGPT account id")?;

        Ok(CodexToken {
            id_token,
            access_token: response.access_token.clone(),
            refresh_token,
            account_id,
            expires: Self::token_expires(&response.access_token),
        })
    }

    pub fn token_expires(access_token: &str) -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        auth::decode_jwt(access_token)
            .and_then(|claims| claims.get("exp")?.as_i64())
            .filter(|exp| *exp >= 0)
            .unwrap_or(now + ASSUMED_LIFETIME.as_secs() as i64)
    }

    pub fn account_id(id_token: &str) -> Option<String> {
        auth::decode_jwt(id_token)?
            .get(AUTH_CLAIMS)?
            .get("chatgpt_account_id")?
            .as_str()
            .map(ToString::to_string)
    }

    pub fn needs_refresh(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        now + REFRESH_SKEW.as_secs() as i64 >= self.expires
    }
}

pub struct CodexAuth {
    token: watch::Sender<Option<CodexToken>>,
    token_rx: watch::Receiver<Option<CodexToken>>,
    login_lock: Mutex<()>,
    token_url: String,
    token_path: PathBuf,
}

static SHARED: OnceLock<Result<Arc<CodexAuth>, String>> = OnceLock::new();

impl std::fmt::Debug for CodexAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexAuth")
            .field("logged_in", &self.is_logged_in())
            .field("account_id", &self.account_id())
            .finish_non_exhaustive()
    }
}

impl PartialEq for CodexAuth {
    fn eq(&self, other: &Self) -> bool {
        self.token() == other.token()
            && self.token_url == other.token_url
            && self.token_path == other.token_path
    }
}

impl CodexAuth {
    fn new() -> Result<CodexAuth, Box<dyn Error>> {
        let path = Self::path()?;
        let token = Self::load_token_from(&path)?;
        Ok(Self::at(TOKEN_URL, &path, token))
    }

    pub fn get() -> Result<Arc<CodexAuth>, Box<dyn Error>> {
        match SHARED.get_or_init(|| {
            CodexAuth::new()
                .map(Arc::new)
                .map_err(|err| err.to_string())
        }) {
            Ok(auth) => Ok(auth.clone()),
            Err(err) => Err(err.clone().into()),
        }
    }

    pub fn at(token_url: &str, token_path: &Path, token: Option<CodexToken>) -> CodexAuth {
        let (token, token_rx) = watch::channel(token);
        CodexAuth {
            token,
            token_rx,
            login_lock: Mutex::new(()),
            token_url: token_url.to_string(),
            token_path: token_path.to_path_buf(),
        }
    }

    pub fn path() -> Result<PathBuf, Box<dyn Error>> {
        Dirs::auth_file("codex")
    }

    pub fn load_token_from(path: &Path) -> Result<Option<CodexToken>, Box<dyn Error>> {
        auth::load_token_file(path, "Codex")
    }

    pub async fn save_token(&self) -> Result<(), Box<dyn Error>> {
        Self::save_token_to(&self.token_path, self.token().as_ref())
    }

    pub fn save_token_to(path: &Path, token: Option<&CodexToken>) -> Result<(), Box<dyn Error>> {
        let token = token.ok_or(NOT_LOGGED_IN)?;
        auth::save_token_file(path, token)
    }

    pub fn token(&self) -> Option<CodexToken> {
        self.token_rx.borrow().clone()
    }

    pub fn account_id(&self) -> Option<String> {
        self.token_rx
            .borrow()
            .as_ref()
            .map(|token| token.account_id.clone())
    }

    pub fn is_logged_in(&self) -> bool {
        self.token_rx.borrow().is_some()
    }

    pub fn set_token(&self, token: Option<CodexToken>) {
        self.token.send_replace(token);
    }

    /// Run the OAuth authorization code + PKCE flow and store the token.
    pub async fn login(&self) -> Result<(), Box<dyn Error>> {
        let oauth = OAuth::new(authorize_url, LOOPBACK_ADDR).await?;

        println!();
        println!("Sign in to ChatGPT to use your Codex subscription:");
        println!("Open this URL in a browser to continue:");
        println!();
        println!("{}", oauth.url);
        println!();

        let code = oauth.await_callback().await?;
        let _guard = self.login_lock.lock().await;
        let client = CodexTokenRequester::new(&self.token_url)?;
        let token = client
            .authorization_code(&code, &oauth.code_verifier)
            .await?;

        self.set_token(Some(token));
        self.save_token().await?;
        println!(
            "Logged in to Codex. Tokens saved to {}",
            self.token_path.display()
        );

        Ok(())
    }

    pub async fn refresh(&self) -> Result<(), Box<dyn Error>> {
        let _guard = self.login_lock.lock().await;
        let current = self.token().ok_or(NOT_LOGGED_IN)?;
        if !current.needs_refresh() {
            return Ok(());
        }

        let client = CodexTokenRequester::new(&self.token_url)?;
        let token = match client.refresh_token(&current).await {
            Ok(token) => token,
            Err(err) => {
                if err
                    .downcast_ref::<TokenError>()
                    .is_some_and(TokenError::is_permanent)
                {
                    self.forget().await;
                }

                return Err(err);
            }
        };

        self.set_token(Some(token));
        self.save_token().await
    }

    async fn forget(&self) {
        self.set_token(None);
        if let Err(err) = std::fs::remove_file(&self.token_path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("Unable to remove {}: {}", self.token_path.display(), err);
        }
    }

    pub async fn access_token(&self) -> Result<String, Box<dyn Error>> {
        let token = self.token().ok_or(NOT_LOGGED_IN)?;
        if !token.needs_refresh() {
            return Ok(token.access_token);
        }

        self.refresh().await?;
        self.token()
            .map(|token| token.access_token)
            .ok_or_else(|| NOT_LOGGED_IN.into())
    }
}

pub fn authorize_url(code_verifier: &str, state: &str) -> String {
    let authorize = format!("{CODEX_ISSUER}{AUTHORIZE_PATH}");
    let mut url = Url::parse(&authorize).expect("authorize url is a valid constant");
    url.query_pairs_mut()
        .append_pair("client_id", CODEX_CLIENT_ID)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", SCOPE)
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("originator", ORIGINATOR);
    auth::append_pkce_params(&mut url, code_verifier, state);

    url.to_string()
}

struct CodexTokenRequester {
    http: reqwest::Client,
    token_url: String,
}

impl CodexTokenRequester {
    pub fn new(token_url: &str) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()?,
            token_url: token_url.to_string(),
        })
    }

    async fn authorization_code(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<CodexToken, Box<dyn Error>> {
        let form = [
            ("client_id", CODEX_CLIENT_ID),
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", code_verifier),
        ];

        let response = self.post_token(&form).await?;
        CodexToken::new(&response, None)
    }

    async fn refresh_token(&self, current: &CodexToken) -> Result<CodexToken, Box<dyn Error>> {
        let form = [
            ("client_id", CODEX_CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", &current.refresh_token),
        ];

        let response = self.post_token(&form).await?;
        CodexToken::new(&response, Some(current))
    }

    async fn post_token(&self, form: &[(&str, &str)]) -> Result<TokenResponse, Box<dyn Error>> {
        let response = self.http.post(&self.token_url).form(&form).send().await?;
        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            return Err(TokenError {
                message: describe_error("codex token endpoint", status, &body),
                status,
                code: auth::error_code(&body),
            }
            .into());
        }

        Ok(serde_json::from_str(&body)?)
    }
}
