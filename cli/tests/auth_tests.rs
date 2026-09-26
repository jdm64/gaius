use base64::Engine;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use gaius::auth::*;
use gaius::auth_codex::*;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

const UNUSED_ENDPOINT: &str = "http://127.0.0.1:1/oauth/token";

fn token_with_exp(exp: i64) -> CodexToken {
    let claims = format!("{{\"exp\":{exp}}}");
    let payload = URL_SAFE_NO_PAD.encode(claims);
    CodexToken {
        id_token: String::new(),
        access_token: format!("header.{payload}.signature"),
        refresh_token: String::new(),
        account_id: String::new(),
        expires: exp,
    }
}

fn expires_in(offset: i64) -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + offset
}

#[test]
fn fresh_token_does_not_need_refresh() {
    let token = token_with_exp(expires_in(3600));
    assert!(!token.needs_refresh());
}

#[test]
fn expiring_token_needs_refresh() {
    let token = token_with_exp(expires_in(60));
    assert!(token.needs_refresh());
}

#[test]
fn expired_token_needs_refresh() {
    let token = token_with_exp(expires_in(-60));
    assert!(token.needs_refresh());
}

#[test]
fn token_without_exp_uses_stored_expiry() {
    let mut token = token_with_exp(0);
    token.access_token = "not-a-jwt".to_string();
    token.expires = expires_in(7 * 24 * 60 * 60);
    assert!(!token.needs_refresh());

    token.expires = 0;
    assert!(token.needs_refresh());
}

#[test]
fn an_expiry_past_2038_is_kept_as_given() {
    // The stored expiry was an `i32` once and every token was clamped to its
    // ceiling. From 2038-01-19 on, that made `needs_refresh` true for every
    // token forever, so each request for one went to the token endpoint to
    // exchange a token that was not yet due.
    for exp in [2_147_483_648, 4_102_444_800] {
        let payload = URL_SAFE_NO_PAD.encode(format!("{{\"exp\":{exp}}}"));
        let expires = CodexToken::token_expires(&format!("header.{payload}.signature"));
        assert_eq!(expires, exp, "the expiry was clamped to {expires}");

        let token = CodexToken {
            expires,
            ..token_with_exp(0)
        };
        assert!(!token.needs_refresh());
    }
}

#[test]
fn extracts_account_id_from_id_token() {
    let claims = format!(
        "{{\"{}\":{{\"chatgpt_account_id\":\"acct-123\"}}}}",
        AUTH_CLAIMS
    );
    let b64 = URL_SAFE_NO_PAD.encode(claims);
    let id_token = format!("header.{}.signature", b64);

    assert_eq!(
        CodexToken::account_id(&id_token).as_deref(),
        Some("acct-123")
    );
    assert_eq!(CodexToken::account_id("garbage"), None);
}

#[test]
fn pkce_challenge_matches_rfc_7636_example() {
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    assert_eq!(code_challenge(verifier), expected);
}

#[test]
fn authorize_url_carries_pkce_and_registered_redirect() {
    let verifier = "verifier-for-the-test";
    let url = url::Url::parse(&authorize_url(verifier, "state-123")).unwrap();
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

    assert_eq!(url.path(), "/oauth/authorize");
    assert_eq!(params["response_type"], "code");
    assert_eq!(params["client_id"], CODEX_CLIENT_ID);
    assert_eq!(params["redirect_uri"], REDIRECT_URI);
    assert_eq!(params["scope"], "openid profile email offline_access");
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["code_challenge"], code_challenge(verifier));
    assert_eq!(params["state"], "state-123");
}

#[test]
fn registered_redirect_is_pinned_to_the_loopback_port() {
    assert_eq!(REDIRECT_URI, "http://localhost:1455/auth/callback");

    let redirect = url::Url::parse(REDIRECT_URI).unwrap();
    let port = redirect.port().unwrap();
    assert_eq!(LOOPBACK_ADDR, format!("localhost:{port}"));
    assert!(
        redirect
            .host_str()
            .is_some_and(|host| host.ends_with("localhost"))
    );
}

#[test]
fn parses_authorization_code_from_callback() {
    let callback = OAuth::parse_callback("/auth/callback?code=abc123&state=xyz").unwrap();

    assert_eq!(callback.code.as_deref(), Some("abc123"));
    assert_eq!(callback.state.as_deref(), Some("xyz"));
    assert_eq!(callback.error, None);
}

#[test]
fn percent_escapes_in_callback_are_decoded() {
    let callback = OAuth::parse_callback("/auth/callback?code=a%2Fb%2Bc&state=x%20y").unwrap();

    assert_eq!(callback.code.as_deref(), Some("a/b+c"));
    assert_eq!(callback.state.as_deref(), Some("x y"));
}

#[test]
fn ignores_loopback_requests_that_are_not_the_callback() {
    assert_eq!(OAuth::parse_callback("/favicon.ico"), None);
    assert_eq!(OAuth::parse_callback("/auth/callback"), None);
    assert_eq!(OAuth::parse_callback("/auth/callback?state=xyz"), None);
    assert_eq!(OAuth::parse_callback("garbage request"), None);
}

#[test]
fn describes_a_refused_authorization() {
    let with_description =
        OAuth::parse_callback("/auth/callback?error=access_denied&error_description=User+declined")
            .unwrap();
    let bare = OAuth::parse_callback("/auth/callback?error=access_denied").unwrap();

    assert_eq!(with_description.code, None);
    assert_eq!(
        with_description.error.as_deref(),
        Some("access_denied: User declined")
    );
    assert_eq!(bare.error.as_deref(), Some("access_denied"));
}

#[test]
fn a_refusal_may_carry_only_a_description() {
    // `error` is optional when `error_description` is sent, and a refusal that
    // is read as "not our request" leaves the user waiting out the login
    // timeout on a 404 page.
    let callback =
        OAuth::parse_callback("/auth/callback?error_description=User+declined").expect("refusal");

    assert_eq!(callback.code, None);
    assert_eq!(callback.error.as_deref(), Some("User declined"));
}

#[tokio::test]
async fn loopback_server_returns_the_authorization_code() {
    let (code, responses) =
        loopback_login(&["/favicon.ico", "/auth/callback?code=code-1&state={state}"]).await;

    assert_eq!(code.as_deref(), Ok("code-1"));
    let answered = responses.join("\n");
    assert!(answered.contains("200 OK"), "{}", answered);
    assert!(answered.contains("close this tab"), "{}", answered);
}

#[tokio::test]
async fn loopback_server_ignores_a_callback_for_another_login() {
    // Ending the wait on a foreign callback would let anything that can reach
    // the port cancel a sign in, so it is answered and then ignored.
    let (code, responses) = loopback_login(&[
        "/auth/callback?code=not-ours&state=other",
        "/auth/callback?code=code-1&state={state}",
    ])
    .await;

    assert_eq!(code.as_deref(), Ok("code-1"));
    assert!(responses[0].contains("400 Bad Request"), "{}", responses[0]);
    assert!(responses[1].contains("200 OK"), "{}", responses[1]);
}

#[tokio::test]
async fn loopback_server_ignores_a_refusal_for_another_login() {
    let (code, responses) = loopback_login(&[
        "/auth/callback?error=access_denied&state=other",
        "/auth/callback?code=code-1&state={state}",
    ])
    .await;

    assert_eq!(code.as_deref(), Ok("code-1"));
    assert!(responses[0].contains("400 Bad Request"), "{}", responses[0]);
}

#[tokio::test]
async fn loopback_server_reports_a_refused_authorization() {
    let (code, _) = loopback_login(&["/auth/callback?error=access_denied"]).await;
    let err = code.unwrap_err();

    assert!(err.contains("access_denied"), "{err}");
}

#[tokio::test]
async fn loopback_server_reports_a_refusal_that_only_has_a_description() {
    let (code, responses) =
        loopback_login(&["/auth/callback?error_description=User+declined"]).await;
    let err = code.unwrap_err();

    assert!(err.contains("User declined"), "{err}");
    assert!(responses[0].contains("400 Bad Request"), "{}", responses[0]);
}

#[tokio::test]
async fn a_silent_peer_does_not_hold_the_login() {
    // Anything on the machine can reach the loopback port, and a peer that
    // connected without sending anything used to park the accept loop for the
    // rest of the login: the callback the user was waiting for then sat
    // unread until the whole timeout ran out, and the error blamed the
    // browser for a sign in that had in fact succeeded.
    let oauth = OAuth::new(authorize_url, "127.0.0.1:0")
        .await
        .expect("bind loopback listener");
    let port = oauth.local_addr().expect("listener address").port();
    let state = state_of(&oauth.url);

    let server =
        tokio::spawn(async move { oauth.await_callback().await.map_err(|err| err.to_string()) });

    // Held open for the length of the test, and deliberately never written to:
    // dropping it sends the FIN that ends a request, which is a different case
    // from the silent one under test here.
    let silent = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let target = format!("/auth/callback?code=code-1&state={state}");
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request = format!("GET {target} HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();

    // Bounded generously: the point is that the answer arrives at all, and it
    // can only arrive after the silent peer has been given up on.
    let mut buf = vec![0u8; 1024];
    let read = tokio::time::timeout(Duration::from_secs(30), stream.read(&mut buf))
        .await
        .expect("the silent peer held the login")
        .expect("read response");
    let response = String::from_utf8_lossy(&buf[..read]).into_owned();
    assert!(response.contains("200 OK"), "{response}");

    let code = wait_for(server)
        .await
        .expect("loopback server did not finish")
        .unwrap();
    assert_eq!(code.as_deref(), Ok("code-1"));

    drop(silent);
}

/// Run an `OAuth` loopback server on an OS-assigned port, sending each target
/// to it in order and returning the resulting code plus the browser response
/// to each request.
///
/// A target containing `{state}` is rewritten with the `state` the server
/// generated, so the happy path echoes it back while other targets can stay
/// deliberately wrong. Every wait is bounded by a timeout to fail the test
/// instead of hanging when something goes wrong.
async fn loopback_login(targets: &[&str]) -> (Result<String, String>, Vec<String>) {
    let oauth = OAuth::new(authorize_url, "127.0.0.1:0")
        .await
        .expect("bind loopback listener");
    let port = oauth.local_addr().expect("listener address").port();

    // `OAuth::new` generated the state and put it in the authorize URL; the
    // callback has to match it.
    let state = state_of(&oauth.url);

    let server =
        tokio::spawn(async move { oauth.await_callback().await.map_err(|err| err.to_string()) });

    let mut responses = Vec::new();
    for target in targets {
        let target = target.replace("{state}", &state);
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let request = format!("GET {target} HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();

        let mut buf = vec![0u8; 1024];
        let read = wait_for(stream.read(&mut buf))
            .await
            .expect("loopback server did not answer")
            .expect("read response");
        responses.push(String::from_utf8_lossy(&buf[..read]).into_owned());
    }

    let code = wait_for(server)
        .await
        .expect("loopback server did not finish")
        .unwrap();

    (code, responses)
}

/// Run `future` with a bound, so a stalled step fails the test rather than hangs.
async fn wait_for<F: Future>(future: F) -> Option<F::Output> {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .ok()
}

#[test]
fn builds_token_from_response_reusing_previous_values() {
    let previous = CodexToken {
        id_token: "old-id".to_string(),
        access_token: "old-access".to_string(),
        refresh_token: "old-refresh".to_string(),
        account_id: "acct-old".to_string(),
        expires: 0,
    };
    let response = TokenResponse {
        access_token: "new-access".to_string(),
        id_token: None,
        refresh_token: Some("new-refresh".to_string()),
    };

    let token = CodexToken::new(&response, Some(&previous)).unwrap();
    assert_eq!(token.id_token, "old-id");
    assert_eq!(token.refresh_token, "new-refresh");
    assert_eq!(token.access_token, "new-access");
    assert_eq!(token.account_id, "acct-old");
}

#[test]
fn login_needs_an_id_token() {
    let response = TokenResponse {
        access_token: "new-access".to_string(),
        id_token: None,
        refresh_token: Some("new-refresh".to_string()),
    };

    assert!(CodexToken::new(&response, None).is_err());
}

#[test]
fn saves_and_loads_token_file() {
    let dir = std::env::temp_dir().join(format!("gaius-auth-test-{}", uuid::Uuid::now_v7()));
    let path = dir.join("auth_codex.json");
    let token = CodexToken {
        id_token: "id".to_string(),
        access_token: "access".to_string(),
        refresh_token: "refresh".to_string(),
        account_id: "acct".to_string(),
        expires: 1_767_225_600,
    };

    CodexAuth::save_token_to(&path, Some(&token)).unwrap();
    assert_eq!(CodexAuth::load_token_from(&path).unwrap(), Some(token));
    assert_eq!(
        CodexAuth::load_token_from(&dir.join("missing.json")).unwrap(),
        None
    );

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_token_file_written_before_the_expiry_widened_still_loads() {
    // `expires` is persisted, and it was an `i32` on disk. A stored number
    // still reads into the wider field, so widening it does not sign an
    // existing login out on upgrade; the value is left exactly as written
    // rather than clamped to the old ceiling.
    let dir = temp_dir_for("auth-legacy-expiry");
    let path = dir.join("auth_codex.json");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &path,
        r#"{"access_token":"access","id_token":"id","refresh_token":"refresh",
            "account_id":"acct","expires":2000000000}"#,
    )
    .unwrap();

    let token = CodexAuth::load_token_from(&path)
        .unwrap()
        .expect("the saved login loads");
    assert_eq!(token.expires, 2_000_000_000);
    assert!(!token.needs_refresh(), "a stored expiry was not honored");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn describes_oauth_errors() {
    let body = r#"{"error":"invalid_grant","error_description":"refresh token expired"}"#;
    let described = describe_error("refresh", reqwest::StatusCode::BAD_REQUEST, body);

    assert!(described.contains("400"), "{}", described);
    assert!(described.contains("invalid_grant"), "{}", described);
    assert!(described.contains("refresh token expired"), "{}", described);
}

fn auth_with(token: CodexToken) -> CodexAuth {
    CodexAuth::at(UNUSED_ENDPOINT, Path::new("unused.json"), Some(token))
}

#[tokio::test]
async fn access_token_uses_stored_token_while_fresh() {
    let mut token = token_with_exp(expires_in(3600));
    token.access_token = "stored-access".to_string();
    token.account_id = "acct-1".to_string();
    let auth = auth_with(token);

    assert!(auth.is_logged_in());
    assert_eq!(auth.account_id().as_deref(), Some("acct-1"));
    assert_eq!(auth.access_token().await.unwrap(), "stored-access");
}

#[tokio::test]
async fn access_token_without_login_points_at_login_command() {
    let auth = CodexAuth::at(UNUSED_ENDPOINT, Path::new("unused.json"), None);
    let err = auth.access_token().await.unwrap_err().to_string();

    assert!(!auth.is_logged_in());
    assert!(err.contains("--login codex"), "{}", err);
}

#[tokio::test]
async fn refresh_without_login_points_at_login_command() {
    let auth = CodexAuth::at(UNUSED_ENDPOINT, Path::new("unused.json"), None);
    let err = auth.refresh().await.unwrap_err().to_string();

    assert!(err.contains("--login codex"), "{}", err);
}

#[test]
fn only_a_refusal_of_the_credentials_counts_as_permanent() {
    let error = |status, code: Option<&str>| TokenError {
        status,
        code: code.map(ToString::to_string),
        message: "refused".to_string(),
    };

    // The grant itself was refused, so the refresh token is spent.
    assert!(error(reqwest::StatusCode::BAD_REQUEST, Some("invalid_grant")).is_permanent());
    assert!(error(reqwest::StatusCode::UNAUTHORIZED, None).is_permanent());

    // Every other client error is a failure of something other than the
    // grant. Dropping the credentials on any of these logs the user out of a
    // login that still works, with only a browser sign in to recover from.
    assert!(!error(reqwest::StatusCode::TOO_MANY_REQUESTS, None).is_permanent());
    assert!(!error(reqwest::StatusCode::SERVICE_UNAVAILABLE, None).is_permanent());
    assert!(!error(reqwest::StatusCode::FORBIDDEN, None).is_permanent());
    assert!(!error(reqwest::StatusCode::REQUEST_TIMEOUT, None).is_permanent());
    assert!(
        !error(
            reqwest::StatusCode::UNPROCESSABLE_ENTITY,
            Some("invalid_scope")
        )
        .is_permanent()
    );
    assert!(!error(reqwest::StatusCode::BAD_REQUEST, None).is_permanent());
    assert!(
        !error(
            reqwest::StatusCode::BAD_REQUEST,
            Some("temporarily_unavailable")
        )
        .is_permanent()
    );
}

#[test]
fn reads_the_oauth_error_code_out_of_a_response() {
    // The code is what tells a dead grant from a request the endpoint did not
    // like, so it has to be read from both shapes an error can arrive in.
    assert_eq!(
        error_code(r#"{"error":"invalid_grant","error_description":"expired"}"#).as_deref(),
        Some("invalid_grant")
    );
    assert_eq!(
        error_code(r#"{"error":{"code":"invalid_grant","message":"expired"}}"#).as_deref(),
        Some("invalid_grant")
    );
    assert_eq!(error_code("not json at all"), None);
    assert_eq!(error_code(r#"{"error_description":"no code here"}"#), None);
}

/// An auth holding a token that is already past its expiry, wired to `endpoint`
/// and to a token file nothing else uses.
fn expired_auth(endpoint: &str, path: &Path) -> CodexAuth {
    let mut token = token_with_exp(expires_in(-60));
    token.access_token = "stored-access".to_string();
    token.refresh_token = "stored-refresh".to_string();
    CodexAuth::at(endpoint, path, Some(token))
}

/// A private temp directory for a test, so its token file can never be
/// confused with a real one and cleanup can never escape it.
fn temp_dir_for(test: &str) -> PathBuf {
    std::env::temp_dir().join(format!("gaius-{test}-{}", uuid::Uuid::now_v7()))
}

#[tokio::test]
async fn a_refused_refresh_drops_the_credentials() {
    let (endpoint, _) = token_endpoint(
        "400 Bad Request",
        r#"{"error":"invalid_grant","error_description":"refresh token expired"}"#,
    )
    .await;
    let dir = temp_dir_for("auth-refused");
    let path = dir.join("auth_codex.json");
    let auth = expired_auth(&endpoint, &path);
    CodexAuth::save_token_to(&path, auth.token().as_ref()).unwrap();

    let err = auth.refresh().await.unwrap_err().to_string();

    // The refusal is reported, and the dead credentials go away instead of
    // being presented again on every later request.
    assert!(err.contains("invalid_grant"), "{err}");
    assert!(!auth.is_logged_in());
    assert!(auth.access_token().await.is_err());
    assert!(
        !path.exists(),
        "{} should have been removed",
        path.display()
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_transient_refresh_failure_keeps_the_credentials() {
    let dir = temp_dir_for("auth-transient");
    let path = dir.join("auth_codex.json");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Nothing is listening now, so the exchange fails before it is judged.
    let endpoint = format!("http://127.0.0.1:{port}/oauth/token");
    let auth = expired_auth(&endpoint, &path);
    CodexAuth::save_token_to(&path, auth.token().as_ref()).unwrap();

    assert!(auth.refresh().await.is_err());

    assert!(auth.is_logged_in());
    assert!(path.exists(), "{} should have been kept", path.display());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_refusal_that_is_not_of_the_credentials_keeps_them() {
    // A 4xx from something in the middle is not a dead grant. Treating every
    // client error as one deleted a login that still worked, and left a
    // browser sign in as the only way to get it back.
    let (endpoint, _) = token_endpoint(
        "403 Forbidden",
        r#"{"error":"forbidden","error_description":"proxy said no"}"#,
    )
    .await;
    let dir = temp_dir_for("auth-forbidden");
    let path = dir.join("auth_codex.json");
    let auth = expired_auth(&endpoint, &path);
    CodexAuth::save_token_to(&path, auth.token().as_ref()).unwrap();

    let err = auth.refresh().await.unwrap_err().to_string();

    assert!(err.contains("403"), "{err}");
    assert!(
        auth.is_logged_in(),
        "a proxy refusal dropped the credentials"
    );
    assert!(path.exists(), "{} should have been kept", path.display());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn concurrent_refreshes_exchange_the_token_once() {
    let (endpoint, exchanges) = token_endpoint(
        "200 OK",
        r#"{"access_token":"new-access","refresh_token":"new-refresh"}"#,
    )
    .await;
    let dir = temp_dir_for("auth-concurrent");
    let path = dir.join("auth_codex.json");
    let auth = Arc::new(expired_auth(&endpoint, &path));

    // The token endpoint rotates the refresh token on each use, so a second
    // exchange started from a stale copy spends a token that no longer works.
    // Every caller below sees the same expired token, and only the one holding
    // the lock may exchange it. (`Box<dyn Error>` is not `Send`, so the
    // refreshes are joined on one task rather than spawned; they still
    // interleave at every await, which is what the lock has to survive.)
    let refreshes = (0..4).map(|_| auth.refresh());
    for refresh in futures::future::join_all(refreshes).await {
        refresh.unwrap();
    }

    assert_eq!(1, exchanges.lock().await.len(), "refreshed more than once");

    let token = auth.token().expect("token");
    assert_eq!(token.access_token, "new-access");
    assert_eq!(token.refresh_token, "new-refresh");
    assert_eq!(
        CodexAuth::load_token_from(&path)
            .unwrap()
            .map(|t| t.refresh_token),
        Some("new-refresh".to_string())
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// A stand-in for the token endpoint that answers every request with `status`
/// and `body`, and records the form bodies it was sent. Returns its URL and
/// the recorded exchanges.
///
/// The reply is delayed a little so that callers racing each other all observe
/// the pre-refresh state, which is what a broken lock would expose.
async fn token_endpoint(
    status: &'static str,
    body: &'static str,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake endpoint");
    let url = format!(
        "http://127.0.0.1:{port}/oauth/token",
        port = listener.local_addr().expect("endpoint address").port()
    );
    let exchanges: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = exchanges.clone();

    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let exchange = read_request_body(&mut stream).await;
            recorded.lock().await.push(exchange);

            tokio::time::sleep(Duration::from_millis(50)).await;
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                 Content-Length: {length}\r\nConnection: close\r\n\r\n{body}",
                length = body.len()
            );
            stream.write_all(response.as_bytes()).await.ok();
        }
    });

    (url, exchanges)
}

/// Read one HTTP request and return its body, so a test can assert on the form
/// fields that were sent.
async fn read_request_body(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0u8; 4096];

    let body_start = loop {
        let read = stream.read(&mut chunk).await.expect("read request");
        if read == 0 {
            return String::new();
        }
        request.extend_from_slice(&chunk[..read]);

        let Some(header_end) = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|end| end + 4)
        else {
            continue;
        };

        let headers = String::from_utf8_lossy(&request[..header_end]).to_lowercase();
        let length: usize = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0);
        if request.len() >= header_end + length {
            break header_end;
        }
    };

    String::from_utf8_lossy(&request[body_start..]).to_string()
}

#[tokio::test]
async fn loopback_server_escapes_a_message_it_did_not_write() {
    // The message comes off the query string and any site can send a browser
    // here, so it must not be able to close the tag it sits in.
    let (code, responses) =
        loopback_login(&["/auth/callback?error=%3Cscript%3Ealert(1)%3C%2Fscript%3E"]).await;

    assert!(
        code.unwrap_err().contains("<script>"),
        "the refusal is reported"
    );
    let page = responses[0].replace(['\r', '\n'], "");
    assert!(!page.contains("<script>"), "{}", page);
    assert!(page.contains("&lt;script&gt;"), "{}", page);
}

#[test]
fn decodes_jwt_claims_with_or_without_padding() {
    // A length that is not a multiple of three, so the padded form really does
    // carry padding and the two cases are not the same string.
    let claims = r#"{"exp":1767225600,"n":123}"#;
    let (padded, unpadded) = (URL_SAFE.encode(claims), URL_SAFE_NO_PAD.encode(claims));
    assert_ne!(padded, unpadded, "this payload cannot show the padded case");
    assert!(padded.ends_with('='), "{padded}");

    for encoded in [padded, unpadded] {
        let token = format!("header.{encoded}.signature");
        assert_eq!(
            decode_jwt(&token).and_then(|value| value.get("exp").cloned()),
            Some(serde_json::json!(1767225600))
        );
    }

    // A payload that is not JSON is no claims at all, rather than claims that
    // happen to be null.
    let unparseable = format!("header.{}.signature", URL_SAFE_NO_PAD.encode("not json"));
    assert_eq!(decode_jwt(&unparseable), None);
    assert_eq!(decode_jwt("no-payload"), None);
}

#[test]
fn a_token_file_is_written_private_and_leaves_no_staging_file() {
    let dir = temp_dir_for("auth-permissions");
    let path = dir.join("auth_codex.json");
    let token = CodexToken {
        id_token: "id".to_string(),
        access_token: "access".to_string(),
        refresh_token: "refresh".to_string(),
        account_id: "acct".to_string(),
        expires: 1_767_225_600,
    };

    CodexAuth::save_token_to(&path, Some(&token)).unwrap();
    assert_eq!(CodexAuth::load_token_from(&path).unwrap(), Some(token));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(0o600, mode, "token file is readable beyond its owner");
    }

    // Only the token file is left behind.
    let entries: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(vec!["auth_codex.json".to_string()], entries);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_new_token_does_not_truncate_the_old_one_in_place() {
    let dir = temp_dir_for("auth-swap");
    let path = dir.join("auth_codex.json");
    let token = |access: &str| CodexToken {
        id_token: "id".to_string(),
        access_token: access.to_string(),
        refresh_token: "refresh".to_string(),
        account_id: "acct".to_string(),
        expires: 1_767_225_600,
    };

    CodexAuth::save_token_to(&path, Some(&token("first"))).unwrap();
    // A refresh rotates the refresh token, so a save interrupted by a crash
    // must not leave a truncated file where a working token used to be. The
    // new token is written beside the old one and swapped in, which a hard
    // link can see: it still reads what the previous save wrote.
    let earlier = dir.join("earlier.json");
    std::fs::hard_link(&path, &earlier).unwrap();

    CodexAuth::save_token_to(&path, Some(&token("second"))).unwrap();

    assert_eq!(
        std::fs::read_to_string(&earlier).unwrap(),
        serde_json::to_string_pretty(&token("first")).unwrap()
    );
    assert_eq!(
        CodexAuth::load_token_from(&path).unwrap(),
        Some(token("second"))
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rewriting_a_token_replaces_a_loose_file_rather_than_keeping_its_mode() {
    let dir = temp_dir_for("auth-replace");
    let path = dir.join("auth_codex.json");
    let token = CodexToken {
        id_token: "id".to_string(),
        access_token: "access".to_string(),
        refresh_token: "refresh".to_string(),
        account_id: "acct".to_string(),
        expires: 1_767_225_600,
    };

    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&path, "{}\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    CodexAuth::save_token_to(&path, Some(&token)).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(0o600, mode, "replaced token kept the old mode");
    }
    assert_eq!(CodexAuth::load_token_from(&path).unwrap(), Some(token));

    std::fs::remove_dir_all(&dir).ok();
}

/// The `state` an `OAuth` put in its authorize URL; a callback has to match it.
fn state_of(url: &str) -> String {
    url::Url::parse(url)
        .expect("parse authorize url")
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .expect("authorize url carries state")
}

#[tokio::test]
async fn readers_never_lose_the_account_id_while_the_token_is_being_replaced() {
    // A reader that is refused by a writer in progress used to come back
    // empty-handed, which dropped the account header from a live request.
    // Reads now take a snapshot, so a stored account id is always readable.
    let dir = temp_dir_for("auth-readers");
    let path = dir.join("auth_codex.json");
    let auth = std::sync::Arc::new(CodexAuth::at(
        UNUSED_ENDPOINT,
        &path,
        Some(account_token("acct-a")),
    ));

    let writer = {
        let auth = auth.clone();
        tokio::spawn(async move {
            for round in 0..2_000 {
                let account = if round % 2 == 0 { "acct-a" } else { "acct-b" };
                auth.set_token(Some(account_token(account)));
                tokio::task::yield_now().await;
            }
        })
    };

    for _ in 0..2_000 {
        let account = auth.account_id();
        assert!(
            matches!(account.as_deref(), Some("acct-a") | Some("acct-b")),
            "read {account:?} while the token was being replaced"
        );
        assert!(auth.is_logged_in());
        tokio::task::yield_now().await;
    }

    writer.await.unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

fn account_token(account_id: &str) -> CodexToken {
    let mut token = token_with_exp(expires_in(3600));
    token.account_id = account_id.to_string();
    token
}

#[tokio::test]
async fn two_auths_differ_when_their_tokens_differ() {
    let one = auth_with(account_token("acct-a"));
    let other = auth_with(account_token("acct-b"));

    // Comparing used to go through a non-blocking read, so two different
    // states could look identical whenever a writer held the lock.
    assert_ne!(one, other);
    assert_eq!(one, auth_with(account_token("acct-a")));
    assert_ne!(
        one,
        CodexAuth::at(UNUSED_ENDPOINT, Path::new("other.json"), None)
    );
}
