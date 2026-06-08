use gaius::config::ProviderConfig;
use gaius::models::{CachedModelDef, ModelDef, ModelPickerRow, Models, RecentModelDef};
use serde_json::json;

fn model(provider: &str, id: &str) -> ModelDef {
    ModelDef {
        provider: provider.to_string(),
        id: id.to_string(),
        context_len: None,
    }
}

fn recent_model(provider: &str, id: &str) -> RecentModelDef {
    RecentModelDef {
        provider: provider.to_string(),
        id: id.to_string(),
    }
}

fn provider_config(url: &str) -> ProviderConfig {
    ProviderConfig {
        name: String::new(),
        kind: "openai".to_string(),
        url: url.to_string(),
        key: String::new(),
    }
}

#[test]
fn model_urls_walks_provider_path_upward() {
    let config = provider_config("https://example.com/api/v1");
    let urls = config.models_list_urls().unwrap();
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
    let config = provider_config("https://example.com/v1/");
    let urls = config.models_list_urls().unwrap();
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
    let config = provider_config("");
    let models = config.extract_model_defs(&json!({
        "data": [
            { "id": "model-a" },
            { "id": "model-b" }
        ]
    }));

    assert_eq!(
        models,
        vec![
            ModelDef {
                provider: String::new(),
                id: "model-a".into(),
                context_len: None
            },
            ModelDef {
                provider: String::new(),
                id: "model-b".into(),
                context_len: None
            },
        ]
    );
}

#[test]
fn extracts_model_arrays() {
    let config = provider_config("");
    let models = config.extract_model_defs(&json!({
        "models": [
            { "name": "model-a" },
            "model-b"
        ]
    }));

    assert_eq!(
        models,
        vec![
            ModelDef {
                provider: String::new(),
                id: "model-a".into(),
                context_len: None
            },
            ModelDef {
                provider: String::new(),
                id: "model-b".into(),
                context_len: None
            },
        ]
    );
}

#[test]
fn model_label_puts_provider_in_brackets() {
    let model = model("local", "llama");

    assert_eq!(model.label(), "llama [local]");
}

#[test]
fn models_cache_serializes_as_provider_model_map() {
    let cache = CachedModelDef::to_cache(&[
        model("provider 2", "model 4"),
        model("provider 1", "model 2"),
        model("provider 1", "model 1"),
        model("provider 2", "model 3"),
    ]);

    assert_eq!(
        serde_json::to_value(cache).unwrap(),
        json!({
            "provider 1": [
                { "id": "model 1", "context_len": null },
                { "id": "model 2", "context_len": null }
            ],
            "provider 2": [
                { "id": "model 3", "context_len": null },
                { "id": "model 4", "context_len": null }
            ]
        })
    );
}

#[test]
fn recent_models_serialize_as_model_array() {
    let recent = vec![
        recent_model("openai", "gpt-5.2"),
        recent_model("local", "llama"),
    ];

    assert_eq!(
        serde_json::to_value(recent).unwrap(),
        json!([
            { "provider": "openai", "id": "gpt-5.2" },
            { "provider": "local", "id": "llama" }
        ])
    );
}

#[test]
fn recent_models_move_existing_model_to_front() {
    let recent = vec![
        recent_model("provider", "model-a"),
        recent_model("provider", "model-b"),
        recent_model("provider", "model-c"),
    ];

    let updated = RecentModelDef::join(&recent, &recent_model("provider", "model-b"));

    assert_eq!(
        updated,
        vec![
            recent_model("provider", "model-b"),
            recent_model("provider", "model-a"),
            recent_model("provider", "model-c"),
        ]
    );
}

#[test]
fn recent_models_truncate_to_eight_entries() {
    let recent: Vec<RecentModelDef> = (0..8)
        .map(|index| recent_model("provider", &format!("model-{index}")))
        .collect();

    let updated = RecentModelDef::join(&recent, &recent_model("provider", "new"));

    assert_eq!(updated.len(), 8);
    assert_eq!(updated.first(), Some(&recent_model("provider", "new")));
    assert_eq!(updated.last(), Some(&recent_model("provider", "model-6")));
}

#[test]
fn recent_models_load_with_cache_enriches_known_models() {
    let recent = vec![
        recent_model("provider", "model-b"),
        recent_model("provider", "stale-model"),
    ];
    let cache = vec![
        model("provider", "model-a"),
        ModelDef {
            provider: "provider".to_string(),
            id: "model-b".to_string(),
            context_len: Some(128_000),
        },
    ];

    let enriched = RecentModelDef::from_cache(&recent, &cache);

    assert_eq!(
        enriched,
        vec![
            ModelDef {
                provider: "provider".to_string(),
                id: "model-b".to_string(),
                context_len: Some(128_000),
            },
            model("provider", "stale-model"),
        ]
    );
}

#[test]
fn model_picker_rows_deduplicate_recent_models_by_identity() {
    let available = vec![ModelDef {
        provider: "provider".to_string(),
        id: "model-a".to_string(),
        context_len: Some(128_000),
    }];
    let recent = vec![model("provider", "model-a")];

    let rows = Models::filter_rows("", &available, &recent);

    assert_eq!(
        rows,
        vec![
            ModelPickerRow::Header("Recent".to_string()),
            ModelPickerRow::RecentModel(model("provider", "model-a")),
        ]
    );
}

#[test]
fn model_picker_rows_put_recent_models_first() {
    let available = vec![
        model("provider", "model-a"),
        model("provider", "model-b"),
        model("provider", "model-c"),
    ];
    let recent = vec![model("provider", "model-b")];

    let rows = Models::filter_rows("", &available, &recent);

    assert_eq!(
        rows,
        vec![
            ModelPickerRow::Header("Recent".to_string()),
            ModelPickerRow::RecentModel(model("provider", "model-b")),
            ModelPickerRow::Separator,
            ModelPickerRow::Model(model("provider", "model-a")),
            ModelPickerRow::Model(model("provider", "model-c")),
        ]
    );
}

#[test]
fn model_picker_rows_include_stale_recent_models() {
    let available = vec![model("provider", "model-a")];
    let recent = vec![model("provider", "stale-model")];

    let rows = Models::filter_rows("", &available, &recent);

    assert_eq!(
        rows,
        vec![
            ModelPickerRow::Header("Recent".to_string()),
            ModelPickerRow::RecentModel(model("provider", "stale-model")),
            ModelPickerRow::Separator,
            ModelPickerRow::Model(model("provider", "model-a")),
        ]
    );
}

#[test]
fn model_picker_rows_hide_empty_filtered_sections() {
    let available = vec![model("provider", "alpha"), model("provider", "beta")];
    let recent = vec![model("provider", "gamma")];

    let rows = Models::filter_rows("alp", &available, &recent);

    assert_eq!(
        rows,
        vec![ModelPickerRow::Model(model("provider", "alpha"))]
    );
}

#[test]
fn model_picker_filter_selects_only_model_rows() {
    let available = vec![model("provider", "alpha"), model("provider", "beta")];
    let recent = vec![model("provider", "beta")];
    let rows = Models::filter_rows("", &available, &recent);

    assert_eq!(
        gaius::input::Input::filter_model_rows("", &rows),
        vec![1, 3]
    );
    assert_eq!(
        gaius::input::Input::filter_model_rows("alp", &rows),
        vec![3]
    );
}

#[test]
fn models_cache_loads_provider_model_map() {
    let cache = serde_json::from_value(json!({
        "provider 1": [
            { "id": "model 2", "context_len": null },
            { "id": "model 1", "context_len": null }
        ],
        "provider 2": [
            { "id": "model 3", "context_len": null }
        ]
    }))
    .unwrap();

    let models = CachedModelDef::to_models(cache);
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
