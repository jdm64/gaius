/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    auth::client::{OAuthResponse, now_epoch, random_token},
    auth::handle::TokenError,
    auth::spec::OAuthSpec,
    dirs::Dirs,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    error::Error,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

const REFRESH_SKEW: Duration = Duration::from_mins(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OAuthFile {
    pub access_token: String,
    pub id_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub account_id: Option<String>,
    pub expires: i64,
}

impl OAuthFile {
    pub fn new(
        spec: &'static OAuthSpec,
        response: &OAuthResponse,
        previous: Option<&OAuthFile>,
        nonce: Option<&str>,
    ) -> Result<OAuthFile, Box<dyn Error>> {
        let id_token = response
            .id_token
            .clone()
            .or_else(|| previous.map(|token| token.id_token.clone()))
            .ok_or_else(|| {
                format!(
                    "{} token response did not include an id token",
                    spec.display
                )
            })?;
        if let Some(nonce) = nonce {
            spec.check_nonce(&id_token, nonce)?;
        }

        let refresh_token = response
            .refresh_token
            .clone()
            .or_else(|| previous.map(|token| token.refresh_token.clone()))
            .ok_or_else(|| {
                format!(
                    "{} token response did not include a refresh token",
                    spec.display
                )
            })?;
        let account_id = spec.account_id_from(&id_token, previous)?;

        Ok(OAuthFile {
            id_token,
            access_token: response.access_token.clone(),
            refresh_token,
            account_id,
            expires: response.expires(),
        })
    }

    pub fn load(id: &str) -> Result<Option<Self>, Box<dyn Error>> {
        let path = Dirs::auth_file(id)?;
        Self::load_from(id, path)
    }

    pub fn load_from(id: &str, path: PathBuf) -> Result<Option<Self>, Box<dyn Error>> {
        if !path.exists() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(&path)
            .map_err(|err| format!("Unable to read {}: {}", path.display(), err))?;
        let token = serde_json::from_str(&contents)
            .map_err(|err| format!("Invalid {id} auth file {}: {}", path.display(), err))?;
        Ok(Some(token))
    }

    pub fn save(&self, id: &str) -> Result<(), Box<dyn Error>> {
        let path = Dirs::auth_file(id)?;
        self.save_to(path)
    }

    pub fn save_to(&self, path: PathBuf) -> Result<(), Box<dyn Error>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::write_private(&path, serde_json::to_string_pretty(self)?.as_bytes())
    }

    pub fn delete(id: &str) -> Result<(), Box<dyn Error>> {
        let path = Dirs::auth_file(id)?;
        if let Err(err) = std::fs::remove_file(&path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("Unable to remove {}: {}", path.display(), err);
            Err(err.into())
        } else {
            Ok(())
        }
    }

    fn write_private(path: &Path, contents: &[u8]) -> Result<(), Box<dyn Error>> {
        let staging = path.with_extension(format!("tmp-{}", random_token(8)));
        let written = Self::write_new_private(&staging, contents).and_then(|()| {
            std::fs::rename(&staging, path)?;
            if let Err(err) = Self::sync_parent(path) {
                eprintln!(
                    "Unable to sync the directory holding {}: {err}",
                    path.display()
                );
            }

            Ok(())
        });
        if written.is_err() {
            std::fs::remove_file(&staging).ok();
        }

        written.map_err(|err| format!("Unable to write {}: {}", path.display(), err).into())
    }

    fn sync_parent(path: &Path) -> std::io::Result<()> {
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        std::fs::File::open(parent)?.sync_all()
    }

    fn write_new_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut file = options.open(path)?;
        file.write_all(contents)?;
        file.sync_all()
    }

    pub fn needs_refresh(&self) -> bool {
        now_epoch() + REFRESH_SKEW.as_secs() as i64 >= self.expires
    }
}

pub struct OAuthFileRequester {
    http: reqwest::Client,
    spec: &'static OAuthSpec,
}

impl OAuthFileRequester {
    pub fn new(spec: &'static OAuthSpec) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()?,
            spec,
        })
    }

    pub async fn authorization_code(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
        nonce: Option<&str>,
    ) -> Result<OAuthFile, Box<dyn Error>> {
        let form = [
            ("client_id", self.spec.client_id),
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
        ];

        let response = self.post_token(&form).await?;
        OAuthFile::new(self.spec, &response, None, nonce)
    }

    pub async fn refresh_token(&self, current: &OAuthFile) -> Result<OAuthFile, Box<dyn Error>> {
        let form = [
            ("client_id", self.spec.client_id),
            ("grant_type", "refresh_token"),
            ("refresh_token", &current.refresh_token),
        ];

        let response = self.post_token(&form).await?;
        OAuthFile::new(self.spec, &response, Some(current), None)
    }

    async fn post_token(&self, form: &[(&str, &str)]) -> Result<OAuthResponse, Box<dyn Error>> {
        let response = self
            .http
            .post(self.spec.token_url)
            .form(&form)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            return Err(TokenError {
                message: Self::describe_error(
                    &format!("{} token endpoint", self.spec.display),
                    status,
                    &body,
                ),
                status,
                code: Self::error_code(&body),
            }
            .into());
        }

        Ok(serde_json::from_str(&body)?)
    }

    pub fn describe_error(what: &str, status: reqwest::StatusCode, body: &str) -> String {
        let detail = match Self::error_fields(body) {
            (Some(code), Some(description)) => Some(format!("{code}: {description}")),
            (Some(code), None) => Some(code),
            (None, Some(description)) => Some(description),
            (None, None) => None,
        }
        .unwrap_or_else(|| body.trim().to_string());

        if detail.is_empty() {
            format!("{what} failed with status {status}")
        } else {
            format!("{what} failed with status {status}: {detail}")
        }
    }

    /// The OAuth `error` code an error response carries, e.g. `invalid_grant`.
    pub fn error_code(body: &str) -> Option<String> {
        Self::error_fields(body).0
    }

    fn error_fields(body: &str) -> (Option<String>, Option<String>) {
        let Ok(value) = serde_json::from_str::<Value>(body) else {
            return (None, None);
        };
        let Some(error) = value.get("error") else {
            return (None, None);
        };

        let code = match error {
            Value::String(code) => Some(code.as_str()),
            Value::Object(_) => error.get("code").and_then(Value::as_str),
            _ => None,
        };
        let description = value
            .get("error_description")
            .or_else(|| error.get("message"))
            .and_then(Value::as_str);

        (
            code.map(ToString::to_string),
            description.map(ToString::to_string),
        )
    }
}
