use gaius::{
    agents::AgentDefinition,
    diff_view::{DiffHunk, DiffLine, DiffLineKind, DiffView},
    harness::{Harness, HarnessEvent, is_rate_limit_error, is_webc_rate_limit},
    token_usage::{TokenUsageLedger, TokenUsageSpan},
};
use genai::Error as GenaiError;
use genai::ModelIden;
use genai::adapter::AdapterKind;
use genai::chat::{
    ChatMessage, ContentPart, CustomPart, MessageContent, ToolCall, ToolResponse, Usage,
};
use genai::webc::Error as WebcError;
use reqwest::{StatusCode, header::HeaderMap};
use serde_json::json;

fn basic_agent() -> AgentDefinition {
    AgentDefinition {
        name: "basic".to_string(),
        prompt: String::new(),
    }
}

fn replay_events(messages: Vec<ChatMessage>) -> Vec<HarnessEvent> {
    let mut events = Vec::new();
    let usage = TokenUsageLedger::default();
    Harness::replay_messages(&messages, &usage, |event| events.push(event));
    events
}

#[test]
fn interactive_harness_starts_with_session_id() {
    let harness = Harness::new(basic_agent(), None).unwrap();

    assert!(harness.session_id().is_some());
}

#[test]
fn harness_without_session_starts_without_session_id() {
    let harness = Harness::new_without_session(basic_agent()).unwrap();

    assert!(harness.session_id().is_none());
}

#[test]
fn replay_reasoning_content_before_assistant_text() {
    let events = replay_events(vec![
        ChatMessage::assistant("visible").with_reasoning_content(Some("thinking".to_string())),
    ]);

    assert_eq!(
        events,
        vec![
            HarnessEvent::Thinking("thinking".to_string()),
            HarnessEvent::AgentMessage("visible".to_string()),
        ]
    );
}

#[test]
fn replay_thought_signature_before_assistant_text() {
    let events = replay_events(vec![ChatMessage::assistant(vec![
        ContentPart::ThoughtSignature("signed thought".to_string()),
        ContentPart::Text("visible".to_string()),
    ])]);

    assert_eq!(
        events,
        vec![
            HarnessEvent::Thinking("signed thought".to_string()),
            HarnessEvent::AgentMessage("visible".to_string()),
        ]
    );
}

#[test]
fn replay_assistant_text_unchanged() {
    let events = replay_events(vec![ChatMessage::assistant("visible")]);

    assert_eq!(
        events,
        vec![HarnessEvent::AgentMessage("visible".to_string())]
    );
}

#[test]
fn replay_diff_marker_after_tool_call() {
    let diff = sample_diff();
    let messages = vec![
        ChatMessage::from(vec![ToolCall {
            call_id: "call-1".to_string(),
            fn_name: "edit_file".to_string(),
            fn_arguments: json!({"file_path":"src/lib.rs"}),
            thought_signatures: None,
        }]),
        ChatMessage::tool(MessageContent::from_parts(vec![
            ContentPart::ToolResponse(ToolResponse::new("call-1", "File edited successfully")),
            ContentPart::Custom(CustomPart {
                model_iden: None,
                data: json!({
                    "kind": "diff_view",
                    "version": 1,
                    "file_path": diff.file_path,
                    "hunks": diff.hunks,
                }),
            }),
        ])),
    ];

    let events = replay_events(messages);

    assert_eq!(
        events,
        vec![
            HarnessEvent::ToolCall {
                name: "edit_file".to_string(),
                arguments: json!({"file_path":"src/lib.rs"}).to_string(),
                result: "File edited successfully".to_string(),
                error: false,
            },
            HarnessEvent::DiffView(sample_diff()),
        ]
    );
}

#[test]
fn replay_tool_error_marker_sets_error_flag() {
    let messages = vec![
        ChatMessage::from(vec![ToolCall {
            call_id: "call-1".to_string(),
            fn_name: "search".to_string(),
            fn_arguments: json!({"query": "rust"}),
            thought_signatures: None,
        }]),
        ChatMessage::tool(MessageContent::from_parts(vec![
            ContentPart::ToolResponse(ToolResponse::new("call-1", "something failed")),
            ContentPart::Custom(CustomPart {
                model_iden: None,
                data: json!({ "tool_error": true }),
            }),
        ])),
    ];

    let events = replay_events(messages);

    assert_eq!(
        events,
        vec![HarnessEvent::ToolCall {
            name: "search".to_string(),
            arguments: json!({"query":"rust"}).to_string(),
            result: "something failed".to_string(),
            error: true,
        },]
    );
}

#[test]
fn replay_tool_error_marker_absent_defaults_false() {
    let messages = vec![
        ChatMessage::from(vec![ToolCall {
            call_id: "call-1".to_string(),
            fn_name: "search".to_string(),
            fn_arguments: json!({"query": "rust"}),
            thought_signatures: None,
        }]),
        ChatMessage::tool(MessageContent::from_parts(vec![ContentPart::ToolResponse(
            ToolResponse::new("call-1", "ok"),
        )])),
    ];

    let events = replay_events(messages);

    assert_eq!(
        events,
        vec![HarnessEvent::ToolCall {
            name: "search".to_string(),
            arguments: json!({"query":"rust"}).to_string(),
            result: "ok".to_string(),
            error: false,
        },]
    );
}

#[test]
fn token_usage_records_initial_prompt_as_baseline() {
    let mut ledger = TokenUsageLedger::default();
    let spans = ledger.record(
        1,
        1,
        &Usage {
            prompt_tokens: Some(100),
            completion_tokens: Some(25),
            total_tokens: Some(125),
            ..Usage::default()
        },
        None,
    );

    assert_eq!(spans.len(), 2);
    assert_eq!(
        spans[0],
        TokenUsageSpan {
            start: 0,
            end: 1,
            prompt: Some(100),
            response: None,
        }
    );
    assert_eq!(
        spans[1],
        TokenUsageSpan {
            start: 1,
            end: 2,
            prompt: None,
            response: Some(25),
        }
    );
}

fn sample_diff() -> DiffView {
    DiffView {
        file_path: "src/lib.rs".to_string(),
        hunks: vec![DiffHunk {
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 1,
            lines: vec![
                DiffLine {
                    kind: DiffLineKind::Delete,
                    old_line: Some(1),
                    new_line: None,
                    text: "old".to_string(),
                    missing_newline: false,
                },
                DiffLine {
                    kind: DiffLineKind::Insert,
                    old_line: None,
                    new_line: Some(1),
                    text: "new".to_string(),
                    missing_newline: false,
                },
            ],
        }],
    }
}

#[test]
fn token_usage_records_prompt_delta_for_message_range() {
    let mut ledger = TokenUsageLedger::default();
    ledger.record(
        1,
        1,
        &Usage {
            prompt_tokens: Some(100),
            completion_tokens: Some(25),
            ..Usage::default()
        },
        None,
    );

    let spans = ledger.record(
        4,
        4,
        &Usage {
            prompt_tokens: Some(210),
            completion_tokens: Some(50),
            ..Usage::default()
        },
        None,
    );

    assert_eq!(
        spans[0],
        TokenUsageSpan {
            start: 1,
            end: 4,
            prompt: Some(110),
            response: None,
        }
    );
    assert_eq!(
        spans[1],
        TokenUsageSpan {
            start: 4,
            end: 5,
            prompt: None,
            response: Some(50),
        }
    );
}

fn webc_429() -> WebcError {
    WebcError::ResponseFailedStatus {
        status: StatusCode::TOO_MANY_REQUESTS,
        body: String::new(),
        headers: Box::new(HeaderMap::new()),
    }
}

#[test]
fn webc_rate_limit_true_for_429() {
    assert!(is_webc_rate_limit(&webc_429()));
}

#[test]
fn webc_rate_limit_false_for_other_status() {
    let err = WebcError::ResponseFailedStatus {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        body: String::new(),
        headers: Box::new(HeaderMap::new()),
    };
    assert!(!is_webc_rate_limit(&err));
}

#[test]
fn webc_rate_limit_false_for_non_http_error() {
    let err = WebcError::ResponseFailedNotJson {
        content_type: "text/plain".to_string(),
        body: String::new(),
    };
    assert!(!is_webc_rate_limit(&err));
}

#[test]
fn genai_rate_limit_true_for_http_429() {
    let err: GenaiError = GenaiError::HttpError {
        status: StatusCode::TOO_MANY_REQUESTS,
        canonical_reason: "Too Many Requests".to_string(),
        body: String::new(),
    };
    assert!(is_rate_limit_error(&err));
}

#[test]
fn genai_rate_limit_false_for_http_500() {
    let err: GenaiError = GenaiError::HttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        canonical_reason: "Internal Server Error".to_string(),
        body: String::new(),
    };
    assert!(!is_rate_limit_error(&err));
}

#[test]
fn genai_rate_limit_true_for_web_adapter_call_429() {
    let err: GenaiError = GenaiError::WebAdapterCall {
        adapter_kind: AdapterKind::OpenAI,
        webc_error: webc_429(),
    };
    assert!(is_rate_limit_error(&err));
}

#[test]
fn genai_rate_limit_true_for_web_model_call_429() {
    let err: GenaiError = GenaiError::WebModelCall {
        model_iden: ModelIden::new(AdapterKind::OpenAI, "gpt-4o"),
        webc_error: webc_429(),
    };
    assert!(is_rate_limit_error(&err));
}

#[test]
fn genai_rate_limit_true_for_web_stream_429() {
    let err: GenaiError = GenaiError::WebStream {
        model_iden: ModelIden::new(AdapterKind::OpenAI, "gpt-4o"),
        cause: "stream error".to_string(),
        error: Box::new(GenaiError::HttpError {
            status: StatusCode::TOO_MANY_REQUESTS,
            canonical_reason: "Too Many Requests".to_string(),
            body: String::new(),
        }),
    };
    assert!(is_rate_limit_error(&err));
}

#[test]
fn genai_rate_limit_false_for_non_genai_error() {
    let err = std::io::Error::new(std::io::ErrorKind::Other, "not a genai error");
    assert!(!is_rate_limit_error(&err));
}
