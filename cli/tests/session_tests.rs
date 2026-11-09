use gaius::session::{Session, SessionFile};
use genai::{
    ModelIden,
    adapter::AdapterKind,
    chat::{ChatMessage, ContentPart, MessageContent},
};
use serde_json::json;
use std::{env, path::PathBuf};

#[test]
fn session_round_trips_messages_with_custom_model_data() {
    let custom_data = json!({
        "type": "reasoning.text",
        "text": "All renamed successfully.",
        "format": "unknown",
        "index": 0
    });
    let custom_part = ContentPart::from_custom(
        custom_data.clone(),
        Some(ModelIden::new(
            AdapterKind::OpenAI,
            "xiaomi/mimo-v2.6-flash",
        )),
    );
    let messages = vec![
        ChatMessage::user("Please rename the files."),
        ChatMessage::assistant(MessageContent::from_parts(vec![custom_part])),
    ];

    let proj_file = env::current_dir()
        .unwrap_or_else(|_| PathBuf::new())
        .join("test.mpk");
    let session = Session::new();
    let session_file = SessionFile {
        proj_dir: proj_file.clone(),
        messages,
        ..SessionFile::default()
    };

    session
        .serialize_session(&proj_file, session_file.as_ref())
        .expect("save session");
    let loaded = Session::deserialize_session(&proj_file, true).expect("load session");
    std::fs::remove_file(&proj_file).expect("delete test session");

    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].role, genai::chat::ChatRole::User);
    assert_eq!(loaded.messages[1].role, genai::chat::ChatRole::Assistant);

    let loaded_custom = serde_json::to_value(&loaded.messages[1].content).unwrap();
    assert_eq!(loaded_custom[0]["Custom"]["data"], custom_data);
    assert_eq!(
        loaded_custom[0]["Custom"]["model_iden"]["model_name"]["Shared"],
        "xiaomi/mimo-v2.6-flash"
    );
}
