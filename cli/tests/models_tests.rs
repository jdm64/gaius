use gaius::models::{AvailableModel, ModelPickerRow};
use serde_json::json;

fn model(provider: &str, id: &str) -> AvailableModel {
    AvailableModel {
        provider: provider.to_string(),
        id: id.to_string(),
    }
}

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
    let model = model("local", "llama");

    assert_eq!(model.label(), "llama [local]");
}

#[test]
fn models_cache_serializes_as_provider_model_map() {
    let cache = gaius::models::cache_from_models(&[
        model("provider 2", "model 4"),
        model("provider 1", "model 2"),
        model("provider 1", "model 1"),
        model("provider 2", "model 3"),
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
fn recent_models_serialize_as_model_array() {
    let recent = vec![model("openai", "gpt-5.2"), model("local", "llama")];

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
        model("provider", "model-a"),
        model("provider", "model-b"),
        model("provider", "model-c"),
    ];

    let updated = gaius::models::recent_models_with_model(&recent, &model("provider", "model-b"));

    assert_eq!(
        updated,
        vec![
            model("provider", "model-b"),
            model("provider", "model-a"),
            model("provider", "model-c"),
        ]
    );
}

#[test]
fn recent_models_truncate_to_eight_entries() {
    let recent: Vec<AvailableModel> = (0..8)
        .map(|index| model("provider", &format!("model-{index}")))
        .collect();

    let updated = gaius::models::recent_models_with_model(&recent, &model("provider", "new"));

    assert_eq!(updated.len(), 8);
    assert_eq!(updated.first(), Some(&model("provider", "new")));
    assert_eq!(updated.last(), Some(&model("provider", "model-6")));
}

#[test]
fn model_picker_rows_put_recent_models_first() {
    let available = vec![
        model("provider", "model-a"),
        model("provider", "model-b"),
        model("provider", "model-c"),
    ];
    let recent = vec![model("provider", "model-b")];

    let rows = gaius::models::model_picker_rows("", &available, &recent);

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

    let rows = gaius::models::model_picker_rows("", &available, &recent);

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

    let rows = gaius::models::model_picker_rows("alp", &available, &recent);

    assert_eq!(
        rows,
        vec![ModelPickerRow::Model(model("provider", "alpha"))]
    );
}

#[test]
fn model_picker_filter_selects_only_model_rows() {
    let available = vec![model("provider", "alpha"), model("provider", "beta")];
    let recent = vec![model("provider", "beta")];
    let rows = gaius::models::model_picker_rows("", &available, &recent);

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
