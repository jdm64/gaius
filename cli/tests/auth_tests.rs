use base64::Engine;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use gaius::auth_client::*;
use gaius::auth_file::{OAuthFile, OAuthFileRequester};
use gaius::auth_handle::*;
use gaius::auth_spec::{CODEX, GROK, Nonce, OAuthKind, OAuthSpec, OPENAI_CLAIMS, Redirect};
use gaius::dirs::Dirs;
use std::error::Error;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use url::form_urlencoded;

/// The login tests bind the ports the providers registered redirects on, so
/// they take turns rather than fighting over them.
static LOGIN_PORTS: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

/// A form value as it goes on the wire.
fn encoded(value: &str) -> String {
    form_urlencoded::Serializer::new(String::new())
        .append_pair("", value)
        .finish()
        .trim_start_matches('=')
        .to_string()
}

fn token_with_exp(exp: i64) -> OAuthFile {
    let claims = format!("{{\"exp\":{exp}}}");
    let payload = URL_SAFE_NO_PAD.encode(claims);
    OAuthFile {
        id_token: String::new(),
        access_token: format!("header.{payload}.signature"),
        refresh_token: String::new(),
        account_id: None,
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
        let response = OAuthResponse {
            access_token: format!("header.{payload}.signature"),
            id_token: None,
            refresh_token: None,
            expires_in: None,
        };
        let expires = response.token_expires();
        assert_eq!(expires, exp, "the expiry was clamped to {expires}");

        let token = OAuthFile {
            expires,
            ..token_with_exp(0)
        };
        assert!(!token.needs_refresh());
    }
}

/// A signed-looking token carrying `claims`, as the id token of a response.
fn id_token_with(claims: &str) -> String {
    let payload = URL_SAFE_NO_PAD.encode(claims);
    format!("header.{payload}.signature")
}

/// A response naming `id_token`, from a provider with no account claim.
fn response_with_id_token(id_token: &str) -> OAuthResponse {
    OAuthResponse {
        access_token: "new-access".to_string(),
        id_token: Some(id_token.to_string()),
        refresh_token: Some("new-refresh".to_string()),
        expires_in: None,
    }
}

#[test]
fn extracts_account_id_from_id_token() {
    // Codex namespaces its own claims and wants the account back on every
    // request, so the id token is read for it.
    let claims = format!(
        "{{\"{}\":{{\"chatgpt_account_id\":\"acct-123\"}}}}",
        OPENAI_CLAIMS
    );
    let token = OAuthFile::new(
        &CODEX,
        &response_with_id_token(&id_token_with(&claims)),
        None,
        None,
    )
    .expect("build codex token");

    assert_eq!(token.account_id.as_deref(), Some("acct-123"));
    // Codex requires one; without it there is no account to send back.
    let missing = OAuthFile::new(&CODEX, &response_with_id_token("garbage"), None, None);
    assert!(missing.is_err(), "an id token with no account was accepted");
}

#[test]
fn a_provider_with_no_account_claim_needs_no_account() {
    // Grok's id token carries no account, and asking for one would make every
    // grok login fail.
    let token = OAuthFile::new(&GROK, &response_with_id_token("garbage"), None, None)
        .expect("build grok token");

    assert_eq!(token.account_id, None);
}

#[test]
fn pkce_challenge_matches_rfc_7636_example() {
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    assert_eq!(OAuthSpec::code_challenge(verifier), expected);
}

/// The authorize URL a provider's spec produces for `redirect_uri`.
///
/// `nonce` is only sent for a provider that uses one, so the tests below can
/// pass the same value for every provider and still see the right URL.
fn authorize_url_for(spec: &'static OAuthSpec, redirect_uri: &str, nonce: &str) -> String {
    let nonce = (spec.nonce == Nonce::Required).then_some(nonce);
    spec.authorize_url("verifier-for-the-test", "state-123", redirect_uri, nonce)
}

#[test]
fn every_authorize_url_carries_pkce_and_its_own_registration() {
    for kind in OAuthKind::ALL {
        let spec = kind.spec();
        let redirect_uri = format!(
            "http://{}:{}{}",
            spec.redirect.host, 1234, spec.redirect.path
        );
        let url = url::Url::parse(&authorize_url_for(spec, &redirect_uri, "nonce-abc"))
            .unwrap_or_else(|_| panic!("{} authorize url is a url", spec.display));
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

        assert_eq!(params["client_id"], spec.client_id, "{}", spec.id);
        assert_eq!(params["redirect_uri"], redirect_uri, "{}", spec.id);
        assert_eq!(params["scope"], spec.scope, "{}", spec.id);
        assert_eq!(params["response_type"], "code", "{}", spec.id);
        assert_eq!(params["code_challenge_method"], "S256", "{}", spec.id);
        assert_eq!(
            params["code_challenge"],
            OAuthSpec::code_challenge("verifier-for-the-test"),
            "{}",
            spec.id
        );
        assert_eq!(params["state"], "state-123", "{}", spec.id);

        // Each provider's endpoints, and every parameter it needs on top of
        // the ones the flow sends anyway.
        assert_eq!(
            format!(
                "{}://{}{}",
                url.scheme(),
                url.host_str().unwrap(),
                url.path()
            ),
            spec.authorize_url,
            "{}",
            spec.id
        );
        for (key, value) in spec.authorize_extra {
            assert_eq!(params[*key], *value, "{} is missing {key}", spec.id);
        }
    }
}

#[test]
fn a_nonce_is_sent_only_where_one_is_required() {
    // xAI checks the nonce it was sent against the id token it returns, so a
    // login without one is refused there; codex never sends one.
    let grok = url::Url::parse(&authorize_url_for(
        &GROK,
        "http://127.0.0.1:1/callback",
        "nonce-abc",
    ))
    .expect("grok authorize url is a url");
    let codex = url::Url::parse(&authorize_url_for(
        &CODEX,
        "http://localhost:1455/auth/callback",
        "nonce-abc",
    ))
    .expect("codex authorize url is a url");

    let nonce_of = |url: &url::Url| {
        url.query_pairs()
            .find(|(key, _)| key == "nonce")
            .map(|(_, value)| value.into_owned())
    };

    assert_eq!(nonce_of(&grok).as_deref(), Some("nonce-abc"));
    assert_eq!(nonce_of(&codex), None);
}

#[test]
fn an_id_token_for_another_login_is_rejected() {
    // The id token comes back with the token response, and this is the one place
    // both it and the nonce that was sent are in hand.
    let returned = id_token_with(r#"{"nonce":"someone-elses-login"}"#);
    let err = OAuthFile::new(
        &GROK,
        &response_with_id_token(&returned),
        None,
        Some("our-login"),
    )
    .expect_err("an id token issued for another login was accepted");

    assert!(
        err.to_string().contains("not issued for this login"),
        "{err}"
    );

    let ours = id_token_with(r#"{"nonce":"our-login"}"#);
    assert!(
        OAuthFile::new(
            &GROK,
            &response_with_id_token(&ours),
            None,
            Some("our-login")
        )
        .is_ok(),
        "the id token for this login was rejected"
    );
}

#[test]
fn codex_is_pinned_to_the_loopback_port_it_registered() {
    // The redirect is registered with a fixed port, so a fallback would send
    // the browser somewhere the authorization was never made for.
    const { assert!(!CODEX.redirect.fallback_port) };
    assert_eq!(
        CODEX.redirect.uri(CODEX.redirect.port),
        "http://localhost:1455/auth/callback"
    );

    // Grok's port is only a preference, and its redirect has to name whichever
    // port the listener ended up on.
    const { assert!(GROK.redirect.fallback_port) };
    assert_eq!(GROK.redirect.uri(56121), "http://127.0.0.1:56121/callback");
    assert_eq!(GROK.redirect.uri(45678), "http://127.0.0.1:45678/callback");
}

#[test]
fn each_provider_keeps_its_token_in_its_own_file() {
    let paths: Vec<String> = OAuthKind::ALL
        .iter()
        .map(|kind| kind.spec().path().unwrap().display().to_string())
        .collect();

    assert_eq!(paths.len(), 2, "the providers share a token file");
    assert_ne!(paths[0], paths[1]);
    assert!(paths[0].ends_with("auth_codex.json"), "{}", paths[0]);
    assert!(paths[1].ends_with("auth_grok.json"), "{}", paths[1]);
}

#[test]
fn parses_authorization_code_from_callback() {
    let callback = Callback::parse("/auth/callback?code=abc123&state=xyz").unwrap();

    assert_eq!(callback.code.as_deref(), Some("abc123"));
    assert_eq!(callback.state.as_deref(), Some("xyz"));
    assert_eq!(callback.error, None);
}

#[test]
fn percent_escapes_in_callback_are_decoded() {
    let callback = Callback::parse("/auth/callback?code=a%2Fb%2Bc&state=x%20y").unwrap();

    assert_eq!(callback.code.as_deref(), Some("a/b+c"));
    assert_eq!(callback.state.as_deref(), Some("x y"));
}

#[test]
fn ignores_loopback_requests_that_are_not_the_callback() {
    assert_eq!(Callback::parse("/favicon.ico"), None);
    assert_eq!(Callback::parse("/auth/callback"), None);
    assert_eq!(Callback::parse("/auth/callback?state=xyz"), None);
    assert_eq!(Callback::parse("garbage request"), None);
}

#[test]
fn describes_a_refused_authorization() {
    let with_description =
        Callback::parse("/auth/callback?error=access_denied&error_description=User+declined")
            .unwrap();
    let bare = Callback::parse("/auth/callback?error=access_denied").unwrap();

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
        Callback::parse("/auth/callback?error_description=User+declined").expect("refusal");

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
    let oauth = OAuthClient::new(loopback_url(&CODEX), &["127.0.0.1:0"])
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

/// The authorize URL builder a test hands the loopback listener. It only has to
/// be a real one: the callback has to match the `state` it puts in.
fn loopback_url(spec: &'static OAuthSpec) -> impl Fn(&str, &str, &std::net::SocketAddr) -> String {
    move |verifier, state, addr| {
        spec.authorize_url(verifier, state, &spec.redirect.uri(addr.port()), None)
    }
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
    let oauth = OAuthClient::new(loopback_url(&CODEX), &["127.0.0.1:0"])
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
    let previous = OAuthFile {
        id_token: "old-id".to_string(),
        access_token: "old-access".to_string(),
        refresh_token: "old-refresh".to_string(),
        account_id: Some("acct-old".to_string()),
        expires: 0,
    };
    let response = OAuthResponse {
        access_token: "new-access".to_string(),
        id_token: None,
        refresh_token: Some("new-refresh".to_string()),
        expires_in: None,
    };

    let token = OAuthFile::new(&CODEX, &response, Some(&previous), None).unwrap();
    assert_eq!(token.id_token, "old-id");
    assert_eq!(token.refresh_token, "new-refresh");
    assert_eq!(token.access_token, "new-access");
    assert_eq!(token.account_id.as_deref(), Some("acct-old"));
}

#[test]
fn login_needs_an_id_token() {
    let response = OAuthResponse {
        access_token: "new-access".to_string(),
        id_token: None,
        refresh_token: Some("new-refresh".to_string()),
        expires_in: None,
    };

    assert!(OAuthFile::new(&CODEX, &response, None, None).is_err());
}

fn load_token_from(spec: &OAuthSpec, path: &Path) -> Result<Option<OAuthFile>, Box<dyn Error>> {
    if !path.exists() {
        return Ok(None);
    }

    let contents = std::fs::read_to_string(path)
        .map_err(|err| format!("Unable to read {}: {}", path.display(), err))?;
    let token = serde_json::from_str(&contents).map_err(|err| {
        format!(
            "Invalid {} auth file {}: {}",
            spec.display,
            path.display(),
            err
        )
    })?;
    Ok(Some(token))
}

#[test]
fn saves_and_loads_token_file() {
    let dir = std::env::temp_dir().join(format!("gaius-auth-test-{}", uuid::Uuid::now_v7()));
    let path = dir.join("auth_codex.json");
    let token = OAuthFile {
        id_token: "id".to_string(),
        access_token: "access".to_string(),
        refresh_token: "refresh".to_string(),
        account_id: Some("acct".to_string()),
        expires: 1_767_225_600,
    };

    let _ = token.save_to(path.clone());
    assert_eq!(load_token_from(&CODEX, &path).unwrap(), Some(token));
    assert_eq!(
        load_token_from(&CODEX, &dir.join("missing.json")).unwrap(),
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
    //
    // `account_id` also changed from a plain string to an optional one, to
    // serve providers that have no account. A string on disk still reads into
    // it, so an existing codex login keeps the account it sends on every
    // request.
    let dir = temp_dir_for("auth-legacy-expiry");
    let path = dir.join("auth_codex.json");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &path,
        r#"{"access_token":"access","id_token":"id","refresh_token":"refresh",
            "account_id":"acct","expires":2000000000}"#,
    )
    .unwrap();

    let token = load_token_from(&CODEX, &path)
        .unwrap()
        .expect("the saved login loads");
    assert_eq!(token.expires, 2_000_000_000);
    assert_eq!(token.account_id.as_deref(), Some("acct"));
    assert!(!token.needs_refresh(), "a stored expiry was not honored");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn describes_oauth_errors() {
    let body = r#"{"error":"invalid_grant","error_description":"refresh token expired"}"#;
    let described =
        OAuthFileRequester::describe_error("refresh", reqwest::StatusCode::BAD_REQUEST, body);

    assert!(described.contains("400"), "{}", described);
    assert!(described.contains("invalid_grant"), "{}", described);
    assert!(described.contains("refresh token expired"), "{}", described);
}

fn auth_with(token: OAuthFile) -> OAuthHandle {
    OAuthHandle::at(&CODEX, Some(token))
}

#[tokio::test]
async fn access_token_uses_stored_token_while_fresh() {
    let mut token = token_with_exp(expires_in(3600));
    token.access_token = "stored-access".to_string();
    token.account_id = Some("acct-1".to_string());
    let auth = auth_with(token);

    assert!(auth.is_logged_in());
    assert_eq!(auth.account_id().as_deref(), Some("acct-1"));
    assert_eq!(auth.access_token().await.unwrap(), "stored-access");
}

#[tokio::test]
async fn access_token_without_login_points_at_login_command() {
    let auth = OAuthHandle::at(&CODEX, None);
    let err = auth.access_token().await.unwrap_err().to_string();

    assert!(!auth.is_logged_in());
    assert!(err.contains("--login codex"), "{}", err);
}

#[tokio::test]
async fn refresh_without_login_points_at_login_command() {
    let auth = OAuthHandle::at(&CODEX, None);
    let err = auth.refresh().await.unwrap_err().to_string();

    assert!(err.contains("--login codex"), "{}", err);
}

/// An auth holding a token that is still valid, wired to `endpoint` and to a
/// token file nothing else uses.
fn valid_auth(id: &'static str, endpoint: String) -> OAuthHandle {
    let endpoint: &'static str = Box::leak(endpoint.into_boxed_str());
    let mut token = token_with_exp(expires_in(3600));
    token.access_token = "stored-access".to_string();
    token.refresh_token = "stored-refresh".to_string();
    token.account_id = Some("acct-1".to_string());
    let spec = leak_codex_spec(id, endpoint);
    OAuthHandle::at(spec, Some(token))
}

#[tokio::test]
async fn refresh_leaves_a_valid_token_alone() {
    let (endpoint, exchanges) = token_endpoint(
        "200 OK",
        r#"{"access_token":"new-access","refresh_token":"new-refresh"}"#,
    )
    .await;
    let auth = valid_auth("test-refresh-skips", endpoint);

    auth.refresh().await.unwrap();

    assert!(
        exchanges.lock().await.is_empty(),
        "a valid token should not be exchanged"
    );
    assert_eq!(auth.token().unwrap().access_token, "stored-access");
}

/// The whole point of the command: get a new access token whether or not the
/// stored one has expired.
#[tokio::test]
async fn refresh_now_exchanges_a_token_that_is_still_valid() {
    let (endpoint, exchanges) = token_endpoint(
        "200 OK",
        r#"{"access_token":"new-access","refresh_token":"new-refresh"}"#,
    )
    .await;
    let auth = valid_auth("test-refresh-now", endpoint);
    let path = Dirs::auth_file(auth.spec().id).unwrap();
    auth.token().map(|t| t.save_to(path.clone()));

    assert!(!auth.token().unwrap().needs_refresh());
    auth.refresh_now().await.unwrap();

    assert_eq!(1, exchanges.lock().await.len(), "should exchange once");
    let token = auth.token().expect("token");
    assert_eq!(token.access_token, "new-access");
    assert_eq!(
        load_token_from(&CODEX, &path)
            .unwrap()
            .map(|t| t.access_token),
        Some("new-access".to_string())
    );

    OAuthFile::delete(auth.spec().id).ok();
}

#[tokio::test]
async fn refresh_now_without_login_points_at_login_command() {
    let auth = OAuthHandle::at(&CODEX, None);
    let err = auth.refresh_now().await.unwrap_err().to_string();

    assert!(err.contains("--login codex"), "{}", err);
}

/// Forcing is a request the user asked for, so a refusal still drops the spent
/// credentials rather than leaving a token that cannot be used.
#[tokio::test]
async fn a_refused_forced_refresh_drops_the_credentials() {
    let (endpoint, _) = token_endpoint(
        "400 Bad Request",
        r#"{"error":"invalid_grant","error_description":"refresh token expired"}"#,
    )
    .await;
    let auth = valid_auth("test-refresh-now-refused", endpoint);
    let path = Dirs::auth_file(auth.spec().id).unwrap();
    auth.token().map(|t| t.save_to(path.clone()));

    assert!(auth.refresh_now().await.is_err());
    assert!(
        !auth.is_logged_in(),
        "a refused grant should log the user out"
    );
    assert!(
        !path.exists(),
        "{} should have been removed",
        path.display()
    );

    OAuthFile::delete(auth.spec().id).ok();
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
        OAuthFileRequester::error_code(
            r#"{"error":"invalid_grant","error_description":"expired"}"#
        )
        .as_deref(),
        Some("invalid_grant")
    );
    assert_eq!(
        OAuthFileRequester::error_code(r#"{"error":{"code":"invalid_grant","message":"expired"}}"#)
            .as_deref(),
        Some("invalid_grant")
    );
    assert_eq!(OAuthFileRequester::error_code("not json at all"), None);
    assert_eq!(
        OAuthFileRequester::error_code(r#"{"error_description":"no code here"}"#),
        None
    );
}

/// An auth holding a token that is already past its expiry, wired to `endpoint`
/// and to a token file nothing else uses.
fn expired_auth(id: &'static str, endpoint: String) -> OAuthHandle {
    let endpoint: &'static str = Box::leak(endpoint.into_boxed_str());
    let mut token = token_with_exp(expires_in(-60));
    token.access_token = "stored-access".to_string();
    token.refresh_token = "stored-refresh".to_string();
    // Codex names an account in its id token and sends it back on every request,
    // so a stored codex login always has one to carry.
    token.account_id = Some("acct-1".to_string());
    let spec = leak_codex_spec(id, endpoint);
    OAuthHandle::at(spec, Some(token))
}

/// Leak a `Codex`-like spec with a custom id and token URL, so tests can point
/// at a mock endpoint without needing a field on the handle.
fn leak_codex_spec(id: &'static str, token_url: &'static str) -> &'static OAuthSpec {
    Box::leak(Box::new(OAuthSpec {
        id,
        display: "Test",
        client_id: CODEX.client_id,
        scope: CODEX.scope,
        authorize_url: CODEX.authorize_url,
        token_url,
        authorize_extra: CODEX.authorize_extra,
        nonce: CODEX.nonce,
        redirect: Redirect {
            host: CODEX.redirect.host,
            port: CODEX.redirect.port,
            path: CODEX.redirect.path,
            fallback_port: CODEX.redirect.fallback_port,
        },
        account_id_claim: CODEX.account_id_claim,
        sign_in_msg: CODEX.sign_in_msg,
        paste_code: false,
    }))
}

/// Leak a `Grok`-like spec with a custom id and token URL.
fn leak_grok_spec(id: &'static str, token_url: &'static str) -> &'static OAuthSpec {
    Box::leak(Box::new(OAuthSpec {
        id,
        display: "Grok",
        client_id: GROK.client_id,
        scope: GROK.scope,
        authorize_url: GROK.authorize_url,
        token_url,
        authorize_extra: GROK.authorize_extra,
        nonce: GROK.nonce,
        redirect: Redirect {
            host: GROK.redirect.host,
            port: GROK.redirect.port,
            path: GROK.redirect.path,
            fallback_port: GROK.redirect.fallback_port,
        },
        account_id_claim: GROK.account_id_claim,
        sign_in_msg: GROK.sign_in_msg,
        paste_code: true,
    }))
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
    let auth = expired_auth("test-refused", endpoint);
    let path = Dirs::auth_file(auth.spec().id).unwrap();
    auth.token().map(|t| t.save_to(path.clone()));

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

    OAuthFile::delete(auth.spec().id).ok();
}

#[tokio::test]
async fn a_transient_refresh_failure_keeps_the_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Nothing is listening now, so the exchange fails before it is judged.
    let endpoint = format!("http://127.0.0.1:{port}/oauth/token");
    let auth = expired_auth("test-transient", endpoint);
    let path = Dirs::auth_file(auth.spec().id).unwrap();
    auth.token().map(|t| t.save_to(path.clone()));

    assert!(auth.refresh().await.is_err());

    assert!(auth.is_logged_in());
    assert!(path.exists(), "{} should have been kept", path.display());

    OAuthFile::delete(auth.spec().id).ok();
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
    let auth = expired_auth("test-forbidden", endpoint);
    let path = Dirs::auth_file(auth.spec().id).unwrap();
    auth.token().map(|t| t.save_to(path.clone()));

    let err = auth.refresh().await.unwrap_err().to_string();

    assert!(err.contains("403"), "{err}");
    assert!(
        auth.is_logged_in(),
        "a proxy refusal dropped the credentials"
    );
    assert!(path.exists(), "{} should have been kept", path.display());

    OAuthFile::delete(auth.spec().id).ok();
}

#[tokio::test]
async fn concurrent_refreshes_exchange_the_token_once() {
    let (endpoint, exchanges) = token_endpoint(
        "200 OK",
        r#"{"access_token":"new-access","refresh_token":"new-refresh"}"#,
    )
    .await;
    let auth = Arc::new(expired_auth("test-concurrent", endpoint));
    let path = Dirs::auth_file(auth.spec().id).unwrap();
    auth.token().map(|t| t.save_to(path.clone()));

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
        load_token_from(&CODEX, &path)
            .unwrap()
            .map(|t| t.refresh_token),
        Some("new-refresh".to_string())
    );

    OAuthFile::delete(auth.spec().id).ok();
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
    token_endpoint_returning(status, move || body.to_string()).await
}

/// A stand-in for the token endpoint that answers every request with `status`
/// and whatever `body` builds, for a response that can only be written once the
/// request has been seen.
async fn token_endpoint_returning<F>(
    status: &'static str,
    body: F,
) -> (String, Arc<Mutex<Vec<String>>>)
where
    F: Fn() -> String + Send + 'static,
{
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
            let body = body();
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
    let token = OAuthFile {
        id_token: "id".to_string(),
        access_token: "access".to_string(),
        refresh_token: "refresh".to_string(),
        account_id: Some("acct".to_string()),
        expires: 1_767_225_600,
    };

    let _ = token.save_to(path.clone());
    assert_eq!(load_token_from(&CODEX, &path).unwrap(), Some(token));

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
    let token = |access: &str| OAuthFile {
        id_token: "id".to_string(),
        access_token: access.to_string(),
        refresh_token: "refresh".to_string(),
        account_id: Some("acct".to_string()),
        expires: 1_767_225_600,
    };

    let first = token("first");
    let _ = first.save_to(path.clone());

    // A refresh rotates the refresh token, so a save interrupted by a crash
    // must not leave a truncated file where a working token used to be. The
    // new token is written beside the old one and swapped in, which a hard
    // link can see: it still reads what the previous save wrote.
    let earlier = dir.join("earlier.json");
    std::fs::hard_link(&path, &earlier).unwrap();

    let second = token("second");
    let _ = second.save_to(path.clone());

    assert_eq!(
        std::fs::read_to_string(&earlier).unwrap(),
        serde_json::to_string_pretty(&token("first")).unwrap()
    );
    assert_eq!(
        load_token_from(&CODEX, &path).unwrap(),
        Some(token("second"))
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rewriting_a_token_replaces_a_loose_file_rather_than_keeping_its_mode() {
    let dir = temp_dir_for("auth-replace");
    let path = dir.join("auth_codex.json");
    let token = OAuthFile {
        id_token: "id".to_string(),
        access_token: "access".to_string(),
        refresh_token: "refresh".to_string(),
        account_id: Some("acct".to_string()),
        expires: 1_767_225_600,
    };

    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&path, "{}\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    let _ = token.save_to(path.clone());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(0o600, mode, "replaced token kept the old mode");
    }
    assert_eq!(load_token_from(&CODEX, &path).unwrap(), Some(token));

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
    let auth = std::sync::Arc::new(OAuthHandle::at(&CODEX, Some(account_token("acct-a"))));

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

fn account_token(account_id: &str) -> OAuthFile {
    let mut token = token_with_exp(expires_in(3600));
    token.account_id = Some(account_id.to_string());
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
    assert_ne!(one, OAuthHandle::at(&CODEX, None));
}

#[test]
fn a_stored_account_id_survives_a_refresh_that_returns_no_id_token() {
    // The refresh response is only an access token, so the id token and the
    // account in it have to come from what is already stored.
    let previous = OAuthFile {
        id_token: "old-id".to_string(),
        access_token: "old-access".to_string(),
        refresh_token: "old-refresh".to_string(),
        account_id: Some("acct-old".to_string()),
        expires: 0,
    };
    let response = OAuthResponse {
        access_token: "new-access".to_string(),
        id_token: None,
        refresh_token: Some("new-refresh".to_string()),
        expires_in: None,
    };

    let token = OAuthFile::new(&CODEX, &response, Some(&previous), None).expect("build token");

    assert_eq!(token.id_token, "old-id");
    assert_eq!(token.account_id.as_deref(), Some("acct-old"));
    assert_eq!(token.refresh_token, "new-refresh");
    assert_eq!(token.access_token, "new-access");
}

#[test]
fn the_granted_lifetime_is_preferred_over_the_claim() {
    // xAI sends `expires_in` and a JWT `exp` that can differ by the round trip;
    // what the endpoint said it granted is the better answer.
    let access_token = token_with_exp(expires_in(60)).access_token;
    let response = OAuthResponse {
        access_token,
        id_token: Some(String::new()),
        refresh_token: Some(String::new()),
        expires_in: Some(3600),
    };

    let token = OAuthFile::new(&GROK, &response, None, None).expect("build grok token");

    assert!(!token.needs_refresh());
    assert_eq!(token.expires, expires_in(3600));
}

#[test]
fn a_response_that_grants_no_lifetime_falls_back_to_the_claim() {
    let access_token = token_with_exp(expires_in(3600)).access_token;
    let response = OAuthResponse {
        access_token,
        id_token: Some(String::new()),
        refresh_token: Some(String::new()),
        expires_in: None,
    };

    let token = OAuthFile::new(&GROK, &response, None, None).expect("build grok token");

    assert_eq!(token.expires, expires_in(3600));
    assert!(!token.needs_refresh());
}

#[tokio::test]
async fn a_provider_is_only_matched_by_its_own_name() {
    // `--login` and the configured kind both go through one lookup, so a name
    // that is a prefix of another's cannot select the wrong login.
    assert_eq!(OAuthKind::from_lower_str("codex"), Some(OAuthKind::Codex));
    assert_eq!(OAuthKind::from_lower_str("GROK"), Some(OAuthKind::Grok));
    assert_eq!(OAuthKind::names(), "codex, grok");

    for name in ["", "chatgpt", "xai", "grok-cli", "cod"] {
        assert_eq!(OAuthKind::from_lower_str(name), None, "{name:?} matched");
    }
}

#[test]
fn two_providers_auths_are_never_equal() {
    // They can hold the same token and still not be interchangeable: they are
    // different logins to different providers.
    let token = account_token("acct-1");
    let codex = OAuthHandle::at(&CODEX, Some(token.clone()));
    let grok = OAuthHandle::at(&GROK, Some(token));

    assert_ne!(codex, grok);
}

/// Drive a login the way a browser would: read the URL, send the callback to
/// the port it names, and hand the code back.
async fn browser_visits(login: &Login, code: &str) -> String {
    // Connect to the address the listener actually bound to, not a hardcoded
    // loopback address, so IPv4 and IPv6 agree on both sides of the socket.
    let addr = login.oauth.local_addr().expect("listener has an address");
    let redirect = url::Url::parse(&login.redirect_uri).expect("redirect is a url");
    let state = state_of(&login.oauth.url);
    let code = code.to_string();

    // The wait is on this task; the callback is sent from another so the two
    // do not deadlock.
    let sending = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let target = format!("{}?code={code}&state={state}", redirect.path());
        let mut stream = TcpStream::connect(addr).await.expect("connect");
        let request = format!("GET {target} HTTP/1.1\r\nHost: {addr}\r\n\r\n");
        stream.write_all(request.as_bytes()).await.expect("write");
    });

    let got = login.oauth.await_callback().await;
    sending.await.expect("the browser was never sent");
    got.expect("the login did not get a code")
}

#[tokio::test]
async fn a_grok_login_sends_a_nonce_and_exchanges_with_the_same_redirect() {
    let _ports = LOGIN_PORTS.lock().await;
    // xAI checks the nonce it was sent against the id token it returns, so the
    // whole exchange hinges on the same nonce being in the URL that opened the
    // login and in the check afterwards.
    let nonce_cell: Arc<OnceLock<String>> = Arc::new(OnceLock::new());
    let claims = nonce_cell.clone();
    let (endpoint, exchanges) = token_endpoint_returning("200 OK", move || {
        let nonce = claims.get().map(String::as_str).unwrap_or_default();
        let id_token = id_token_with(&format!(r#"{{"nonce":"{nonce}"}}"#));
        format!(
            r#"{{"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600,"id_token":"{id_token}"}}"#
        )
    })
    .await;

    let endpoint: &'static str = Box::leak(endpoint.into_boxed_str());
    let spec = leak_grok_spec("test-grok-login", endpoint);
    let auth = OAuthHandle::at(spec, None);
    let login = auth.begin_login().await.expect("begin grok login");

    let nonce = login
        .nonce
        .as_deref()
        .expect("grok sends a nonce")
        .to_string();
    nonce_cell.set(nonce.clone()).ok();

    let url = url::Url::parse(&login.oauth.url).expect("login url is a url");
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(params["nonce"], nonce, "the nonce is not in the URL");
    assert_eq!(params["client_id"], GROK.client_id);
    assert_eq!(params["scope"], GROK.scope);
    assert_eq!(params["redirect_uri"], login.redirect_uri);

    // The redirect has to name the port the listener actually got, which is not
    // necessarily the one grok prefers.
    let redirect = url::Url::parse(&login.redirect_uri).expect("redirect is a url");
    assert_eq!(redirect.host_str(), Some(GROK.redirect.host));
    assert_eq!(redirect.path(), GROK.redirect.path);
    // Whatever port the listener got, the browser below reaches it, so the two
    // agree.
    assert!(redirect.port().is_some_and(|port| port > 0), "{redirect}");

    let code = browser_visits(&login, "code-1").await;
    auth.finish_login(&login, &code)
        .await
        .expect("finish grok login");

    let exchange = exchanges.lock().await.join("\n");
    assert!(
        exchange.contains("grant_type=authorization_code"),
        "{exchange}"
    );
    assert!(exchange.contains("code=code-1"), "{exchange}");
    assert!(
        exchange.contains(&format!("client_id={}", GROK.client_id)),
        "{exchange}"
    );
    // The exchange has to repeat the redirect the authorization was made with,
    // or the code is not the one the grant was issued for.
    assert!(
        exchange.contains(&format!("redirect_uri={}", encoded(&login.redirect_uri))),
        "{exchange}"
    );
    assert!(exchange.contains("code_verifier="), "{exchange}");

    let token = auth.token().expect("a token was stored");
    assert_eq!(token.access_token, "new-access");
    assert_eq!(token.refresh_token, "new-refresh");
    assert_eq!(token.account_id, None, "grok's id token names no account");
    assert!(!token.needs_refresh());
    let path = Dirs::auth_file(auth.spec().id).unwrap();
    assert_eq!(load_token_from(auth.spec(), &path).unwrap(), Some(token));

    OAuthFile::delete(auth.spec().id).ok();
}

#[tokio::test]
async fn a_codex_login_sends_no_nonce_and_stores_the_account() {
    let _ports = LOGIN_PORTS.lock().await;
    // The other half of the same flow, for the provider that needs no nonce and
    // whose id token names the account to send back.
    let body = format!(
        r#"{{"access_token":"new-access","refresh_token":"new-refresh","id_token":"{}"}}"#,
        id_token_with(&format!(
            r#"{{"{}":{{"chatgpt_account_id":"acct-1"}}}}"#,
            OPENAI_CLAIMS
        ))
    );
    let (endpoint, exchanges) = token_endpoint_returning("200 OK", move || body.clone()).await;

    let endpoint: &'static str = Box::leak(endpoint.into_boxed_str());
    let spec = leak_codex_spec("test-codex-login", endpoint);
    let auth = OAuthHandle::at(spec, None);
    let login = auth.begin_login().await.expect("begin codex login");

    assert_eq!(login.nonce, None, "codex was sent a nonce");
    let url = url::Url::parse(&login.oauth.url).expect("login url is a url");
    assert!(
        !url.query_pairs().any(|(key, _)| key == "nonce"),
        "the nonce reached the codex authorize url"
    );
    assert_eq!(login.redirect_uri, "http://localhost:1455/auth/callback");

    let code = browser_visits(&login, "code-1").await;
    auth.finish_login(&login, &code)
        .await
        .expect("finish codex login");

    let exchange = exchanges.lock().await.join("\n");
    assert!(exchange.contains("code=code-1"), "{exchange}");
    assert!(
        exchange.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"),
        "{exchange}"
    );

    let token = auth.token().expect("a token was stored");
    assert_eq!(token.account_id.as_deref(), Some("acct-1"));

    OAuthFile::delete(auth.spec().id).ok();
}

#[tokio::test]
async fn a_grok_login_takes_another_port_when_its_preferred_one_is_taken() {
    let _ports = LOGIN_PORTS.lock().await;
    // Grok's registered port is a preference, so a login still has to work when
    // something else is on it.
    let squatter = TcpListener::bind("127.0.0.1:56121")
        .await
        .expect("take grok's preferred port");
    let dir = temp_dir_for("auth-grok-port");
    let auth = OAuthHandle::at(&GROK, None);

    let login = auth.begin_login().await.expect("begin grok login");

    let redirect = url::Url::parse(&login.redirect_uri).expect("redirect is a url");
    assert_ne!(
        redirect.port(),
        Some(56121),
        "the login was sent to the port that was taken"
    );
    assert!(redirect.port().is_some_and(|port| port > 0));

    drop(squatter);
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_codex_login_will_not_move_off_the_port_it_registered() {
    let _ports = LOGIN_PORTS.lock().await;
    // Codex registered its redirect on one port. A fallback would send the
    // browser somewhere the authorization was never made for, so the login has
    // to fail and say which address was taken.
    let _squatter = TcpListener::bind("127.0.0.1:1455")
        .await
        .expect("take codex's registered port");
    let auth = OAuthHandle::at(&CODEX, None);

    let err = auth
        .begin_login()
        .await
        .expect_err("the codex login moved to another port")
        .to_string();

    assert!(err.contains("127.0.0.1:1455"), "{err}");
    assert!(err.contains("Another login may be running"), "{err}");
}

/// The code a login takes from what was copied back from the sign-in page,
/// bounded so a wait that never ends fails the test instead of hanging.
async fn pasted_code(oauth: &OAuthClient, pasted: &str) -> Result<String, String> {
    let mut input: &[u8] = pasted.as_bytes();
    wait_for(oauth.await_code_from(&mut input))
        .await
        .expect("the login never took the paste")
        .map_err(|err| err.to_string())
}

#[tokio::test]
async fn the_end_of_the_paste_leaves_the_browser_wait_running() {
    // stdin ends at once when no terminal is attached. The login must then
    // carry on waiting for the browser alone, rather than treat the closed
    // input as the end of the sign-in.
    let oauth = OAuthClient::new(loopback_url(&CODEX), &["127.0.0.1:0"])
        .await
        .expect("bind loopback listener");
    let port = oauth.local_addr().expect("listener address").port();
    let state = state_of(&oauth.url);
    let mut input: &[u8] = b"";

    let (code, _) = tokio::join!(wait_for(oauth.await_code_from(&mut input)), async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let target = format!("/auth/callback?code=code-5&state={state}");
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let request = format!("GET {target} HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
    });

    let code = code
        .expect("the login never finished")
        .map_err(|err| err.to_string());
    assert_eq!(code.as_deref(), Ok("code-5"));
}

#[tokio::test]
async fn a_grok_login_completes_from_a_pasted_code() {
    // The whole point of the paste: the code the page showed, copied back
    // into the client, has to reach the token endpoint the same way a
    // browser callback does — that request is where the login itself comes
    // from.
    let _ports = LOGIN_PORTS.lock().await;
    let nonce_cell: Arc<OnceLock<String>> = Arc::new(OnceLock::new());
    let claims = nonce_cell.clone();
    let (endpoint, exchanges) = token_endpoint_returning("200 OK", move || {
        let nonce = claims.get().map(String::as_str).unwrap_or_default();
        let id_token = id_token_with(&format!(r#"{{"nonce":"{nonce}"}}"#));
        format!(
            r#"{{"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600,"id_token":"{id_token}"}}"#
        )
    })
    .await;

    let endpoint: &'static str = Box::leak(endpoint.into_boxed_str());
    let spec = leak_grok_spec("test-grok-paste", endpoint);
    let auth = OAuthHandle::at(spec, None);
    let login = auth.begin_login().await.expect("begin grok login");
    nonce_cell
        .set(login.nonce.clone().expect("grok sends a nonce"))
        .ok();

    let code = pasted_code(&login.oauth, "pasted-code-1\n")
        .await
        .expect("paste");
    auth.finish_login(&login, &code)
        .await
        .expect("finish grok login");

    let exchange = exchanges.lock().await.join("\n");
    assert!(
        exchange.contains("grant_type=authorization_code"),
        "{exchange}"
    );
    assert!(exchange.contains("code=pasted-code-1"), "{exchange}");
    assert!(
        exchange.contains(&format!("redirect_uri={}", encoded(&login.redirect_uri))),
        "{exchange}"
    );
    assert!(exchange.contains("code_verifier="), "{exchange}");

    let token = auth.token().expect("a token was stored");
    assert_eq!(token.access_token, "new-access");
    assert_eq!(token.refresh_token, "new-refresh");
    let path = Dirs::auth_file(auth.spec().id).unwrap();
    assert_eq!(load_token_from(auth.spec(), &path).unwrap(), Some(token));

    OAuthFile::delete(auth.spec().id).ok();
}
