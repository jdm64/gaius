/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use base64::{
    Engine as _,
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
};
use rand::Rng;
use serde::Deserialize;
use serde_json::Value;
use std::{
    error::Error,
    net::SocketAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};
use url::{Url, form_urlencoded};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
const ASSUMED_LIFETIME: Duration = Duration::from_hours(5 * 24);
const VERIFIER_BYTES: usize = 32;
const STATE_BYTES: usize = 16;
const MAX_REQUEST: usize = 8 * 1024;

#[derive(Deserialize)]
pub struct OAuthResponse {
    pub access_token: String,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

impl OAuthResponse {
    pub fn expires(&self) -> i64 {
        self.expires_in.map_or_else(
            || self.token_expires(),
            |expires_in| now_epoch().saturating_add(expires_in.min(i64::MAX as u64) as i64),
        )
    }

    pub fn token_expires(&self) -> i64 {
        decode_jwt(&self.access_token)
            .and_then(|claims| claims.get("exp")?.as_i64())
            .filter(|exp| *exp >= 0)
            .unwrap_or(now_epoch() + ASSUMED_LIFETIME.as_secs() as i64)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Callback {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

impl Callback {
    pub fn parse(target: &str) -> Option<Callback> {
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
}

pub struct OAuthClient {
    pub code_verifier: String,
    state: String,
    pub url: String,
    listener: TcpListener,
}

impl OAuthClient {
    pub async fn new<F>(url_callback: F, addrs: &[&str]) -> Result<Self, Box<dyn Error>>
    where
        F: Fn(&str, &str, &SocketAddr) -> String,
    {
        let (listener, addr) = Self::listen(addrs).await?;
        let code_verifier = random_token(VERIFIER_BYTES);
        let state = random_token(STATE_BYTES);
        let url = url_callback(&code_verifier, &state, &addr);

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

    async fn listen(addrs: &[&str]) -> Result<(TcpListener, SocketAddr), Box<dyn Error>> {
        let mut failure = None;
        for addr in addrs {
            match TcpListener::bind(addr).await {
                Ok(listener) => {
                    let addr = listener.local_addr()?;
                    return Ok((listener, addr));
                }
                Err(err) => failure = Some((*addr, err)),
            }
        }

        let (addr, err) =
            failure.ok_or("No loopback address was given to listen on for the login callback")?;
        Err(format!(
            "Unable to listen on {addr} for the login callback ({err}). \
             Another login may be running; close it and try again."
        )
        .into())
    }

    pub async fn await_callback(&self) -> Result<String, Box<dyn Error>> {
        tokio::time::timeout(LOGIN_TIMEOUT, self.callback_code())
            .await
            .map_err(|_| Self::login_timed_out())?
    }

    pub async fn await_code(&self) -> Result<String, Box<dyn Error>> {
        println!("If the page shows a code to copy back instead of redirecting,");
        println!("paste it here and press Enter:");
        println!();

        let mut paste = BufReader::new(tokio::io::stdin());
        tokio::time::timeout(LOGIN_TIMEOUT, self.await_code_from(&mut paste))
            .await
            .map_err(|_| Self::login_timed_out())?
    }

    pub async fn await_code_from<R: AsyncBufRead + Unpin>(
        &self,
        paste: &mut R,
    ) -> Result<String, Box<dyn Error>> {
        let callback = self.callback_code();
        tokio::pin!(callback);
        let mut eof = false;

        loop {
            if eof {
                return callback.as_mut().await;
            }

            let mut line = String::new();
            tokio::select! {
                code = &mut callback => return code,
                read = paste.read_line(&mut line) => match read {
                    Ok(0) | Err(_) => eof = true,
                    Ok(_) if line.trim().is_empty() => {}
                    Ok(_) => return Ok(line.trim().to_string()),
                }
            }
        }
    }

    pub fn login_timed_out() -> Box<dyn Error> {
        format!(
            "Timed out after {} seconds waiting for the browser to finish signing in.",
            LOGIN_TIMEOUT.as_secs()
        )
        .into()
    }

    pub async fn callback_code(&self) -> Result<String, Box<dyn Error>> {
        loop {
            let (mut stream, _) = self.listener.accept().await?;
            let Some(target) = Self::read_target(&mut stream).await else {
                continue;
            };
            let Some(callback) = Callback::parse(&target) else {
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

pub fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn decode_jwt(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let claims = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;

    serde_json::from_slice(&claims).ok()
}
