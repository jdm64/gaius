use gaius::providers::ProviderDef;

fn api_key_provider(url: &str) -> ProviderDef {
    ProviderDef::ApiKey {
        name: String::new(),
        kind: "openai".to_string(),
        url: url.to_string(),
        key: String::new(),
    }
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
