/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    auth_client::{self, random_token},
    auth_file::OAuthFile,
    dirs::Dirs,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::{error::Error, path::PathBuf};
use url::Url;

const NONCE_BYTES: usize = 16;
pub const OPENAI_CLAIMS: &str = "https://api.openai.com/auth";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Nonce {
    None,
    Required,
}

pub struct Redirect {
    pub host: &'static str,
    pub port: u16,
    pub path: &'static str,
    pub fallback_port: bool,
}

impl Redirect {
    pub fn addrs(&self) -> Vec<String> {
        let mut addrs = vec![format!("{}:{}", self.host, self.port)];
        if self.fallback_port {
            addrs.push(format!("{}:0", self.host));
        }

        addrs
    }

    pub fn uri(&self, port: u16) -> String {
        format!("http://{}:{port}{}", self.host, self.path)
    }
}

pub struct OAuthSpec {
    pub id: &'static str,
    pub display: &'static str,
    pub client_id: &'static str,
    pub scope: &'static str,
    pub authorize_url: &'static str,
    pub token_url: &'static str,
    pub authorize_extra: &'static [(&'static str, &'static str)],
    pub nonce: Nonce,
    pub redirect: Redirect,
    pub account_id_claim: Option<(&'static str, &'static str)>,
    pub sign_in_msg: &'static str,
}

pub const CODEX: OAuthSpec = OAuthSpec {
    id: "codex",
    display: "Codex",
    client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
    scope: "openid profile email offline_access",
    authorize_url: "https://auth.openai.com/oauth/authorize",
    token_url: "https://auth.openai.com/oauth/token",
    authorize_extra: &[
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", "codex_cli_rs"),
    ],
    nonce: Nonce::None,
    redirect: Redirect {
        host: "localhost",
        port: 1455,
        path: "/auth/callback",
        fallback_port: false,
    },
    account_id_claim: Some(("https://api.openai.com/auth", "chatgpt_account_id")),
    sign_in_msg: "Sign in to ChatGPT to use your Codex subscription:",
};

/// info pulled from: https://auth.x.ai/.well-known/openid-configuration
pub const GROK: OAuthSpec = OAuthSpec {
    id: "grok",
    display: "Grok",
    client_id: "b1a00492-073a-47ea-816f-4c329264a828",
    scope: "openid profile email offline_access grok-cli:access api:access \
            conversations:read conversations:write",
    authorize_url: "https://auth.x.ai/oauth2/authorize",
    token_url: "https://auth.x.ai/oauth2/token",
    authorize_extra: &[],
    nonce: Nonce::Required,
    redirect: Redirect {
        host: "127.0.0.1",
        port: 56121,
        path: "/callback",
        fallback_port: true,
    },
    account_id_claim: None,
    sign_in_msg: "Sign in to xAI to use your Grok subscription:",
};

impl OAuthSpec {
    pub fn account_id_from(
        &self,
        id_token: &str,
        previous: Option<&OAuthFile>,
    ) -> Result<Option<String>, Box<dyn Error>> {
        let Some((namespace, claim)) = self.account_id_claim else {
            return Ok(previous.and_then(|token| token.account_id.clone()));
        };

        let account_id = auth_client::decode_jwt(id_token)
            .and_then(|claims| {
                claims
                    .get(namespace)
                    .and_then(|claims| claims.get(claim))
                    .and_then(|value| value.as_str())
                    .map(ToString::to_string)
            })
            .or_else(|| previous.and_then(|token| token.account_id.clone()));

        account_id.map(Some).ok_or_else(|| {
            format!("{} token response did not include account id", self.display).into()
        })
    }

    pub fn generate_nonce(&self) -> Option<String> {
        (self.nonce == Nonce::Required).then(|| random_token(NONCE_BYTES))
    }

    pub fn check_nonce(&self, id_token: &str, nonce: &str) -> Result<(), Box<dyn Error>> {
        let returned = auth_client::decode_jwt(id_token)
            .and_then(|claims| claims.get("nonce")?.as_str().map(String::from));
        if returned.as_deref() != Some(nonce) {
            return Err(format!("{} id token was not issued for this login", self.display).into());
        }

        Ok(())
    }

    pub fn not_logged_in(&self) -> String {
        format!(
            "Not logged in to {}. Run 'gaius --login {}' first.",
            self.display, self.id
        )
    }

    pub fn path(&self) -> Result<PathBuf, Box<dyn Error>> {
        Dirs::auth_file(self.id)
    }

    pub fn authorize_url(
        &self,
        code_verifier: &str,
        state: &str,
        redirect_uri: &str,
        nonce: Option<&str>,
    ) -> String {
        let mut url = Url::parse(self.authorize_url)
            .unwrap_or_else(|err| panic!("{} authorize url is not a URL: {err}", self.display));
        url.query_pairs_mut()
            .append_pair("client_id", self.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", self.scope);
        for (key, value) in self.authorize_extra {
            url.query_pairs_mut().append_pair(key, value);
        }
        if let Some(nonce) = nonce {
            url.query_pairs_mut().append_pair("nonce", nonce);
        }
        Self::append_pkce_params(&mut url, code_verifier, state);

        url.to_string()
    }

    /// Append the parameters every authorization code flow sends. Providers add
    /// their own (client id, redirect, scope, ...) around these.
    fn append_pkce_params(url: &mut Url, code_verifier: &str, state: &str) {
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("code_challenge", &Self::code_challenge(code_verifier))
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", state);
    }

    /// The S256 PKCE challenge for `code_verifier`. The plain method is not accepted
    /// by the Codex authorize endpoint.
    pub fn code_challenge(code_verifier: &str) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()))
    }

    pub fn save_token_to(&self, token: Option<&OAuthFile>) -> Result<(), Box<dyn Error>> {
        let token = token.ok_or_else(|| self.not_logged_in())?;
        token.save(self.id)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OAuthKind {
    Codex,
    Grok,
}

impl OAuthKind {
    pub const ALL: [OAuthKind; 2] = [OAuthKind::Codex, OAuthKind::Grok];

    pub fn from_lower_str(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "codex" => Some(OAuthKind::Codex),
            "grok" => Some(OAuthKind::Grok),
            _ => None,
        }
    }

    pub fn spec(self) -> &'static OAuthSpec {
        match self {
            OAuthKind::Codex => &CODEX,
            OAuthKind::Grok => &GROK,
        }
    }

    pub fn id(self) -> &'static str {
        self.spec().id
    }

    pub fn names() -> String {
        OAuthKind::ALL
            .iter()
            .map(|kind| kind.id())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn slot(self) -> usize {
        match self {
            OAuthKind::Codex => 0,
            OAuthKind::Grok => 1,
        }
    }
}
