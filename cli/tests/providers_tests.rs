use gaius::auth::file::OAuthFile;
use gaius::auth::handle::OAuthHandle;
use gaius::auth::spec::{CODEX, GROK, OAuthSpec};
use gaius::config::ProviderConfig;
use gaius::providers::ProviderDef;
use genai::Headers;
use std::error::Error;
use std::sync::Arc;

fn api_key_provider(url: &str) -> ProviderDef {
    ProviderDef::ApiKey {
        name: String::new(),
        kind: "openai".to_string(),
        url: url.to_string(),
        key: String::new(),
    }
}

pub fn empty_auth(spec: &'static OAuthSpec) -> Result<OAuthHandle, Box<dyn Error>> {
    Ok(OAuthHandle::at(spec, None))
}

fn logged_in_auth(spec: &'static OAuthSpec, access_token: &str) -> OAuthHandle {
    OAuthHandle::at(
        spec,
        Some(OAuthFile {
            id_token: String::new(),
            access_token: access_token.to_string(),
            refresh_token: "refresh".to_string(),
            account_id: Some("acct-1".to_string()),
            expires: 1_767_225_600,
        }),
    )
}

#[test]
fn codex_provider_has_no_token_until_logged_in() {
    let provider = ProviderDef::Codex {
        name: "Codex".to_string(),
        auth: Arc::new(empty_auth(&CODEX).expect("empty auth")),
    };

    assert!(!matches!(&provider, ProviderDef::Codex { auth, .. } if auth.is_logged_in()));
    assert_eq!(provider.kind_str(), "codex");
    assert_eq!(provider.name(), "Codex");
}

#[tokio::test]
async fn codex_provider_without_login_points_at_login_command() {
    let provider = ProviderDef::Codex {
        name: "Codex".to_string(),
        auth: Arc::new(empty_auth(&CODEX).expect("empty auth")),
    };

    let err = provider
        .create_client("gpt-5-codex".to_string())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("--login codex"), "{}", err);
}

#[test]
fn codex_model_urls_point_at_chatgpt_backend() {
    let provider = ProviderDef::Codex {
        name: "Codex".to_string(),
        auth: Arc::new(empty_auth(&CODEX).expect("empty auth")),
    };
    let urls: Vec<String> = provider
        .models_list_urls()
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();

    assert_eq!(
        urls,
        vec!["https://chatgpt.com/backend-api/codex/models?client_version=0.155.0"]
    );
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

#[test]
fn codex_providers_share_one_auth() {
    let config = ProviderConfig {
        name: "Codex".to_string(),
        kind: "codex".to_string(),
        url: String::new(),
        key: String::new(),
    };

    // The endpoint rotates the refresh token on every use, so two providers
    // holding separate copies could each exchange the same one and leave the
    // loser (or the token file) with credentials that no longer work.
    let (first, second) = match (ProviderDef::new(&config), ProviderDef::new(&config)) {
        (Ok(first), Ok(second)) => (first, second),
        // Building one reads the saved token, so an unreadable local token
        // file leaves nothing to compare.
        _ => return,
    };
    let (ProviderDef::Codex { auth: one, .. }, ProviderDef::Codex { auth: other, .. }) =
        (&first, &second)
    else {
        panic!("expected codex providers");
    };

    assert!(
        Arc::ptr_eq(one, other),
        "codex providers built separate auths"
    );
}

#[test]
fn codex_requests_carry_the_account_header() {
    let provider = ProviderDef::Codex {
        name: "Codex".to_string(),
        auth: Arc::new(logged_in_auth(&CODEX, "access")),
    };

    let mut headers = Headers::default();
    provider.add_headers(&mut headers);

    let merged = format!("{headers:?}");
    assert!(merged.contains("acct-1"), "{merged}");
    assert!(merged.contains("originator"), "{merged}");
}

#[test]
fn grok_requests_carry_the_client_headers() {
    let provider = ProviderDef::Grok {
        name: "Grok".to_string(),
        auth: Arc::new(logged_in_auth(&GROK, "access")),
    };

    let mut headers = Headers::default();
    provider.add_headers(&mut headers);

    let merged = format!("{headers:?}");
    assert!(merged.contains("grok-shell"), "{merged}");
    assert!(merged.contains("x-grok-client-identifier"), "{merged}");
    assert!(merged.contains("X-XAI-Token-Auth"), "{merged}");
    // Grok's id token has no account to name, so the header codex needs is not
    // sent rather than sent empty.
    assert!(!merged.contains("chatgpt-account-id"), "{merged}");
}

#[tokio::test]
async fn grok_provider_without_login_points_at_its_own_login_command() {
    let provider = ProviderDef::Grok {
        name: "Grok".to_string(),
        auth: Arc::new(empty_auth(&GROK).expect("empty auth")),
    };

    assert_eq!(provider.kind_str(), "grok");
    assert_eq!(provider.name(), "Grok");

    let err = provider
        .create_client("grok-4".to_string())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("--login grok"), "{}", err);
    assert!(!err.contains("--login codex"), "{}", err);
}
