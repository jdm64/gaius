/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    auth_client::OAuthClient,
    auth_file::{OAuthFile, OAuthFileRequester},
    auth_spec::{OAuthKind, OAuthSpec},
};
use std::{
    error::Error,
    sync::{Arc, OnceLock},
};
use tokio::sync::{Mutex, watch};

const INVALID_GRANT: &str = "invalid_grant";

pub struct OAuthHandle {
    spec: &'static OAuthSpec,
    token: watch::Sender<Option<OAuthFile>>,
    token_rx: watch::Receiver<Option<OAuthFile>>,
    login_lock: Mutex<()>,
}

/// One login per provider, kept for the life of the process.
static SHARED: [OnceLock<Result<Arc<OAuthHandle>, String>>; 2] = [OnceLock::new(), OnceLock::new()];

impl std::fmt::Debug for OAuthHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubAuth")
            .field("provider", &self.spec.id)
            .field("logged_in", &self.is_logged_in())
            .field("account_id", &self.account_id())
            .finish_non_exhaustive()
    }
}

impl PartialEq for OAuthHandle {
    fn eq(&self, other: &Self) -> bool {
        self.token() == other.token() && std::ptr::eq(self.spec, other.spec)
    }
}

impl OAuthHandle {
    fn new(spec: &'static OAuthSpec) -> Result<OAuthHandle, Box<dyn Error>> {
        let token = OAuthFile::load(spec.id)?;
        Ok(Self::at(spec, token))
    }

    pub fn get(kind: OAuthKind) -> Result<Arc<OAuthHandle>, Box<dyn Error>> {
        let spec = kind.spec();
        match SHARED[kind.slot()].get_or_init(|| {
            OAuthHandle::new(spec)
                .map(Arc::new)
                .map_err(|err| err.to_string())
        }) {
            Ok(auth) => Ok(auth.clone()),
            Err(err) => Err(err.clone().into()),
        }
    }

    pub fn at(spec: &'static OAuthSpec, token: Option<OAuthFile>) -> OAuthHandle {
        let (token, token_rx) = watch::channel(token);
        OAuthHandle {
            spec,
            token,
            token_rx,
            login_lock: Mutex::new(()),
        }
    }

    pub fn spec(&self) -> &'static OAuthSpec {
        self.spec
    }

    pub fn token(&self) -> Option<OAuthFile> {
        self.token_rx.borrow().clone()
    }

    pub fn account_id(&self) -> Option<String> {
        self.token_rx
            .borrow()
            .as_ref()
            .and_then(|token| token.account_id.clone())
    }

    pub fn is_logged_in(&self) -> bool {
        self.token_rx.borrow().is_some()
    }

    pub fn set_token(&self, token: Option<OAuthFile>) {
        self.token.send_replace(token);
    }

    pub async fn save_token(&self) -> Result<(), Box<dyn Error>> {
        if let Some(token) = self.token() {
            token.save(self.spec.id)?
        }
        Ok(())
    }

    /// Run the OAuth authorization code + PKCE flow and store the token.
    pub async fn login(&self) -> Result<(), Box<dyn Error>> {
        let login = self.begin_login().await?;

        println!();
        println!("{}", self.spec.sign_in_msg);
        println!("Open this URL in a browser to continue:");
        println!();
        println!("{}", login.oauth.url);
        println!();

        let code = if self.spec.paste_code {
            login.oauth.await_code().await?
        } else {
            login.oauth.await_callback().await?
        };
        self.finish_login(&login, &code).await
    }

    pub async fn begin_login(&self) -> Result<Login, Box<dyn Error>> {
        let spec = self.spec;
        let redirect = &spec.redirect;
        let addrs = redirect.addrs();
        let addrs: Vec<&str> = addrs.iter().map(String::as_str).collect();
        let nonce = spec.generate_nonce();

        let oauth = OAuthClient::new(
            |code_verifier, state, addr| {
                let redirect_uri = redirect.uri(addr.port());
                spec.authorize_url(code_verifier, state, &redirect_uri, nonce.as_deref())
            },
            &addrs,
        )
        .await?;

        let redirect_uri = redirect.uri(oauth.local_addr()?.port());
        Ok(Login {
            oauth,
            spec,
            nonce,
            redirect_uri,
        })
    }

    pub async fn finish_login(&self, login: &Login, code: &str) -> Result<(), Box<dyn Error>> {
        let _guard = self.login_lock.lock().await;
        let client = OAuthFileRequester::new(self.spec)?;
        let token = client
            .authorization_code(
                code,
                &login.oauth.code_verifier,
                &login.redirect_uri,
                login.nonce.as_deref(),
            )
            .await?;

        self.set_token(Some(token));
        self.save_token().await?;
        println!("Logged in to {}.", self.spec.display);

        Ok(())
    }

    pub async fn refresh(&self) -> Result<(), Box<dyn Error>> {
        let _guard = self.login_lock.lock().await;
        let current = self.token().ok_or_else(|| self.spec.not_logged_in())?;
        if !current.needs_refresh() {
            return Ok(());
        }

        let client = OAuthFileRequester::new(self.spec)?;
        let token = match client.refresh_token(&current).await {
            Ok(token) => token,
            Err(err) => {
                if err
                    .downcast_ref::<TokenError>()
                    .is_some_and(TokenError::is_permanent)
                {
                    self.forget().await?;
                }

                return Err(err);
            }
        };

        self.set_token(Some(token));
        self.save_token().await
    }

    async fn forget(&self) -> Result<(), Box<dyn Error>> {
        self.set_token(None);
        OAuthFile::delete(self.spec.id)?;
        Ok(())
    }

    pub async fn access_token(&self) -> Result<String, Box<dyn Error>> {
        let token = self.token().ok_or_else(|| self.spec.not_logged_in())?;
        if !token.needs_refresh() {
            return Ok(token.access_token);
        }

        self.refresh().await?;
        Ok(self
            .token()
            .map(|token| token.access_token)
            .ok_or_else(|| self.spec.not_logged_in())?)
    }
}

#[derive(Debug)]
pub struct TokenError {
    pub status: reqwest::StatusCode,
    pub code: Option<String>,
    pub message: String,
}

impl TokenError {
    pub fn is_permanent(&self) -> bool {
        if self.status == reqwest::StatusCode::UNAUTHORIZED {
            return true;
        }

        self.status == reqwest::StatusCode::BAD_REQUEST
            && self.code.as_deref() == Some(INVALID_GRANT)
    }
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for TokenError {}

pub struct Login {
    pub oauth: OAuthClient,
    pub spec: &'static OAuthSpec,
    pub nonce: Option<String>,
    pub redirect_uri: String,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login")
            .field("spec", &self.spec.id)
            .field("nonce", &self.nonce)
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}
