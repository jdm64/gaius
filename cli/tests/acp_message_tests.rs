use agent_client_protocol::v1::schema::{ClientRequest, PromptRequest};
use gaius::acp::message::ACPRequest;
use serde_json::json;
use std::path::Path;

#[test]
fn prompt_v1_merges_text_and_resource_names_and_normalizes_uris() {
    let request: PromptRequest = serde_json::from_value(json!({
        "sessionId": "session",
        "prompt": [
            {"type": "text", "text": "Fix compile errors in "},
            {"type": "resource_link", "name": "main.rs", "uri": "file:///workspace/src/main.rs"},
            {"type": "text", "text": ". Read "},
            {"type": "resource_link", "name": "notes.md", "uri": "file:///home/notes.md"},
            {"type": "text", "text": " for full details."}
        ]
    }))
    .expect("valid v1 prompt request");

    let ACPRequest::Prompt(prompt) = ACPRequest::from(ClientRequest::PromptRequest(request)) else {
        panic!("expected prompt request");
    };
    let prompt_data = prompt.prompt_data().expect("supported prompt blocks");
    let text = prompt_data
        .build(Some(Path::new("/workspace")))
        .expect("prompt should build");

    assert_eq!(
        text,
        concat!(
            "Fix compile errors in `main.rs`. Read `notes.md` for full details.\n\n",
            "---\n",
            "**Attachments**\n",
            "- {\"name\":\"main.rs\",\"uri\":\"src/main.rs\"}\n",
            "- {\"name\":\"notes.md\",\"uri\":\"file:///home/notes.md\"}"
        )
    );
}
