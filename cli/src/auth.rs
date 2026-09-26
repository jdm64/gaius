/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use base64::{
    Engine as _,
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
};
use rand::Rng;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{error::Error, io::Write, net::SocketAddr, path::Path, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::{Url, form_urlencoded};

pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
const VERIFIER_BYTES: usize = 32;
const STATE_BYTES: usize = 16;
const MAX_REQUEST: usize = 8 * 1024;
const INVALID_GRANT: &str = "invalid_grant";

#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Callback {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

pub struct OAuth {
    pub code_verifier: String,
    state: String,
    pub url: String,
    listener: TcpListener,
}

impl OAuth {
    pub async fn new<F>(url_callback: F, bind_addr: &str) -> Result<Self, Box<dyn Error>>
    where
        F: Fn(&str, &str) -> String,
    {
        let listener = Self::bind(bind_addr).await?;
        let code_verifier = random_token(VERIFIER_BYTES);
        let state = random_token(STATE_BYTES);
        let url = url_callback(&code_verifier, &state);

        Ok(Self {
            code_verifier,
            state,
            url,
            listener,
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    async fn bind(addr: &str) -> Result<TcpListener, Box<dyn Error>> {
        TcpListener::bind(addr).await.map_err(|err| {
            format!(
                "Unable to listen on {addr} for the login callback ({err}). \
                     Another login may be running; close it and try again."
            )
            .into()
        })
    }

    pub async fn await_callback(&self) -> Result<String, Box<dyn Error>> {
        tokio::time::timeout(LOGIN_TIMEOUT, self.callback_code())
            .await
            .map_err(|_| {
                format!(
                    "Timed out after {} seconds waiting for the browser to finish signing in.",
                    LOGIN_TIMEOUT.as_secs()
                )
            })?
    }

    async fn callback_code(&self) -> Result<String, Box<dyn Error>> {
        loop {
            let (mut stream, _) = self.listener.accept().await?;
            let Some(target) = Self::read_target(&mut stream).await else {
                continue;
            };
            let Some(callback) = Self::parse_callback(&target) else {
                Self::respond(&mut stream, "404 Not Found", "<html>Not found</html>").await;
                continue;
            };

            if let Some(state) = &callback.state
                && state != &self.state
            {
                Self::respond(
                    &mut stream,
                    "400 Bad Request",
                    &Self::page("Sign in failed", "This callback was not for this login."),
                )
                .await;
                continue;
            }

            if let Some(error) = callback.error {
                Self::respond(
                    &mut stream,
                    "400 Bad Request",
                    &Self::page("Sign in failed", &error),
                )
                .await;
                return Err(format!("Sign in was not completed: {error}").into());
            }

            let (Some(code), Some(_)) = (callback.code, callback.state) else {
                Self::respond(
                    &mut stream,
                    "400 Bad Request",
                    &Self::page("Sign in failed", "This callback was not for this login."),
                )
                .await;
                continue;
            };

            Self::respond(
                &mut stream,
                "200 OK",
                &Self::page("Signed in", "You can close this tab and return to gaius."),
            )
            .await;
            return Ok(code);
        }
    }

    async fn read_target(stream: &mut TcpStream) -> Option<String> {
        tokio::time::timeout(CONNECTION_TIMEOUT, Self::request_target(stream))
            .await
            .unwrap_or_default()
    }

    async fn request_target(stream: &mut TcpStream) -> Option<String> {
        let mut request = Vec::with_capacity(1024);
        let mut chunk = [0u8; 1024];

        loop {
            let read = stream.read(&mut chunk).await.ok()?;
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);

            let complete = request
                .windows(4)
                .last()
                .is_some_and(|window| window == b"\r\n\r\n");
            if complete || request.len() > MAX_REQUEST {
                break;
            }
        }

        let request = String::from_utf8_lossy(&request);
        let request_line = request.lines().next()?;
        Some(request_line.split_whitespace().nth(1)?.to_string())
    }

    pub fn parse_callback(target: &str) -> Option<Callback> {
        let mut callback = Callback::default();
        let mut description = None;

        for (key, value) in Self::callback_query(target)? {
            match key.as_str() {
                "code" => callback.code = Some(value),
                "state" => callback.state = Some(value),
                "error" => callback.error = Some(value),
                "error_description" => description = Some(value),
                _ => {}
            }
        }

        // `error` is optional when `error_description` is sent (RFC 6749
        // 4.1.2.1), and that combination is still a refusal: without this
        // the callback reads as "not our request", the browser gets a 404 and
        // the caller waits out its whole login timeout.
        if let Some(description) = description {
            callback.error = Some(match callback.error.take() {
                Some(error) => format!("{error}: {description}"),
                None => description,
            });
        }

        (callback.code.is_some() || callback.error.is_some()).then_some(callback)
    }

    fn callback_query(target: &str) -> Option<Vec<(String, String)>> {
        let absolute = if target.starts_with('/') {
            None
        } else {
            Some(Url::parse(target).ok()?)
        };

        let query = match &absolute {
            Some(url) => url.query()?,
            None => target.split_once('?').map(|(_, query)| query)?,
        };

        Some(
            form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect(),
        )
    }

    async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.flush().await;
    }

    fn page(title: &str, message: &str) -> String {
        format!(
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{}</title></head>\
             <body><h1>{}</h1><p>{}</p></body></html>",
            Self::escape(title),
            Self::escape(title),
            Self::escape(message)
        )
    }

    fn escape(text: &str) -> String {
        let mut escaped = String::with_capacity(text.len());
        for character in text.chars() {
            match character {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' => escaped.push_str("&gt;"),
                '"' => escaped.push_str("&quot;"),
                '\'' => escaped.push_str("&#39;"),
                _ => escaped.push(character),
            }
        }
        escaped
    }
}

/// A URL-safe random string, used for the PKCE verifier and the `state`.
pub fn random_token(bytes: usize) -> String {
    let mut token = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut token);
    URL_SAFE_NO_PAD.encode(token)
}

/// The S256 PKCE challenge for `code_verifier`. The plain method is not accepted
/// by the Codex authorize endpoint.
pub fn code_challenge(code_verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()))
}

/// Append the parameters every authorization code flow sends. Providers add
/// their own (client id, redirect, scope, ...) around these.
pub fn append_pkce_params(url: &mut Url, code_verifier: &str, state: &str) {
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("code_challenge", &code_challenge(code_verifier))
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);
}

pub fn load_token_file<T: DeserializeOwned>(
    path: &Path,
    what: &str,
) -> Result<Option<T>, Box<dyn Error>> {
    if !path.exists() {
        return Ok(None);
    }

    let contents = std::fs::read_to_string(path)
        .map_err(|err| format!("Unable to read {}: {}", path.display(), err))?;
    let token = serde_json::from_str(&contents)
        .map_err(|err| format!("Invalid {what} auth file {}: {}", path.display(), err))?;
    Ok(Some(token))
}

pub fn save_token_file<T: Serialize>(path: &Path, token: &T) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_private(path, serde_json::to_string_pretty(token)?.as_bytes())
}

fn write_private(path: &Path, contents: &[u8]) -> Result<(), Box<dyn Error>> {
    let staging = path.with_extension(format!("tmp-{}", random_token(8)));
    let written = write_new_private(&staging, contents).and_then(|()| {
        std::fs::rename(&staging, path)?;
        if let Err(err) = sync_parent(path) {
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

/// The OAuth `error` code an error response carries, e.g. `invalid_grant`.
pub fn error_code(body: &str) -> Option<String> {
    error_fields(body).0
}

pub fn describe_error(what: &str, status: reqwest::StatusCode, body: &str) -> String {
    let detail = match error_fields(body) {
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

pub fn decode_jwt(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let claims = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;

    serde_json::from_slice(&claims).ok()
}
