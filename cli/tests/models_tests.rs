use gaius::models::AvailableModel;
use serde_json::json;

#[test]
fn model_urls_walks_provider_path_upward() {
    let urls = gaius::models::list_models_urls("https://example.com/api/v1").unwrap();
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
    let urls = gaius::models::list_models_urls("https://example.com/v1/").unwrap();
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
fn extracts_openai_compatible_models() {
    let ids = gaius::models::extract_model_ids(&json!({
        "data": [
            { "id": "model-a" },
            { "id": "model-b" }
        ]
    }));

    assert_eq!(ids, vec!["model-a", "model-b"]);
}

#[test]
fn extracts_model_arrays() {
    let ids = gaius::models::extract_model_ids(&json!({
        "models": [
            { "name": "model-a" },
            "model-b"
        ]
    }));

    assert_eq!(ids, vec!["model-a", "model-b"]);
}

#[test]
fn model_label_puts_provider_in_brackets() {
    let model = AvailableModel {
        provider: "local".to_string(),
        id: "llama".to_string(),
    };

    assert_eq!(model.label(), "llama [local]");
}

#[test]
fn models_cache_serializes_as_provider_model_map() {
    let cache = gaius::models::cache_from_models(&[
        AvailableModel {
            provider: "provider 2".to_string(),
            id: "model 4".to_string(),
        },
        AvailableModel {
            provider: "provider 1".to_string(),
            id: "model 2".to_string(),
        },
        AvailableModel {
            provider: "provider 1".to_string(),
            id: "model 1".to_string(),
        },
        AvailableModel {
            provider: "provider 2".to_string(),
            id: "model 3".to_string(),
        },
    ]);

    assert_eq!(
        serde_json::to_value(cache).unwrap(),
        json!({
            "provider 1": ["model 1", "model 2"],
            "provider 2": ["model 3", "model 4"]
        })
    );
}

#[test]
fn models_cache_loads_provider_model_map() {
    let cache = serde_json::from_value(json!({
        "provider 1": ["model 2", "model 1"],
        "provider 2": ["model 3"]
    }))
    .unwrap();

    let models = gaius::models::models_from_cache(cache);
    let models: Vec<(String, String)> = models
        .into_iter()
        .map(|model| (model.provider, model.id))
        .collect();

    assert_eq!(
        models,
        vec![
            ("provider 1".to_string(), "model 1".to_string()),
            ("provider 1".to_string(), "model 2".to_string()),
            ("provider 2".to_string(), "model 3".to_string())
        ]
    );
}
