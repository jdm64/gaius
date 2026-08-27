use gaius::{
    agents::AgentDefinition,
    compact::Compact,
    diff_view::{DiffHunk, DiffLine, DiffLineKind, DiffView},
    harness::{Harness, HarnessEvent},
    history_replay,
    models::TokenPrice,
    rate_limit::{is_rate_limit_error, is_webc_rate_limit},
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
    history_replay::replay_messages(&messages, &usage, |event| events.push(event));
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
                start_time: 0,
            },
            HarnessEvent::ToolResult {
                name: "edit_file".to_string(),
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
        vec![
            HarnessEvent::ToolCall {
                name: "search".to_string(),
                arguments: json!({"query":"rust"}).to_string(),
                start_time: 0,
            },
            HarnessEvent::ToolResult {
                name: "search".to_string(),
                result: "something failed".to_string(),
                error: true,
            },
        ]
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
        vec![
            HarnessEvent::ToolCall {
                name: "search".to_string(),
                arguments: json!({"query":"rust"}).to_string(),
                start_time: 0,
            },
            HarnessEvent::ToolResult {
                name: "search".to_string(),
                result: "ok".to_string(),
                error: false,
            },
        ]
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

#[test]
fn genai_rate_limit_true_for_http_400_with_nested_429() {
    let body = serde_json::json!({
        "error": {
            "message": "Provider returned error",
            "code": 400,
            "metadata": {
                "previous_errors": [
                    {"code": 429, "message": "Rate limit exceeded"}
                ]
            }
        }
    });
    let err: GenaiError = GenaiError::HttpError {
        status: StatusCode::BAD_REQUEST,
        canonical_reason: "Bad Request".to_string(),
        body: body.to_string(),
    };
    assert!(is_rate_limit_error(&err));
}

#[test]
fn genai_rate_limit_false_for_http_400_with_nested_400() {
    let body = serde_json::json!({
        "error": {
            "message": "Bad request",
            "code": 400,
            "metadata": {
                "previous_errors": [
                    {"code": 400, "message": "Invalid parameter"}
                ]
            }
        }
    });
    let err: GenaiError = GenaiError::HttpError {
        status: StatusCode::BAD_REQUEST,
        canonical_reason: "Bad Request".to_string(),
        body: body.to_string(),
    };
    assert!(!is_rate_limit_error(&err));
}

#[test]
fn genai_rate_limit_false_for_http_400_without_previous_errors() {
    let body = serde_json::json!({
        "error": {
            "message": "Bad request",
            "code": 400
        }
    });
    let err: GenaiError = GenaiError::HttpError {
        status: StatusCode::BAD_REQUEST,
        canonical_reason: "Bad Request".to_string(),
        body: body.to_string(),
    };
    assert!(!is_rate_limit_error(&err));
}

#[test]
fn genai_rate_limit_false_for_http_400_with_malformed_body() {
    let err: GenaiError = GenaiError::HttpError {
        status: StatusCode::BAD_REQUEST,
        canonical_reason: "Bad Request".to_string(),
        body: "not json".to_string(),
    };
    assert!(!is_rate_limit_error(&err));
}

#[test]
fn genai_rate_limit_false_for_http_400_with_empty_previous_errors() {
    let body = serde_json::json!({
        "error": {
            "metadata": {
                "previous_errors": []
            }
        }
    });
    let err: GenaiError = GenaiError::HttpError {
        status: StatusCode::BAD_REQUEST,
        canonical_reason: "Bad Request".to_string(),
        body: body.to_string(),
    };
    assert!(!is_rate_limit_error(&err));
}

// --- ChatResponse rate-limit tests ---

#[test]
fn genai_rate_limit_true_for_chat_response_with_code_429() {
    let body = json!({
        "code": 429,
        "message": "openai/gpt-5.6-terra is temporarily rate-limited upstream.",
        "metadata": { "error_type": "rate_limit_exceeded" }
    });
    let err: GenaiError = GenaiError::ChatResponse {
        model_iden: ModelIden::new(AdapterKind::OpenAI, "openai/gpt-5.6-terra"),
        body,
    };
    assert!(is_rate_limit_error(&err));
}

#[test]
fn genai_rate_limit_true_for_chat_response_with_code_429_only() {
    // Body with code 429 but no metadata
    let body = json!({
        "code": 429,
        "message": "Rate limit exceeded"
    });
    let err: GenaiError = GenaiError::ChatResponse {
        model_iden: ModelIden::new(AdapterKind::OpenAI, "some-model"),
        body,
    };
    assert!(is_rate_limit_error(&err));
}

#[test]
fn genai_rate_limit_false_for_chat_response_with_other_error() {
    let body = json!({
        "code": 500,
        "message": "Internal server error"
    });
    let err: GenaiError = GenaiError::ChatResponse {
        model_iden: ModelIden::new(AdapterKind::OpenAI, "some-model"),
        body,
    };
    assert!(!is_rate_limit_error(&err));
}

#[test]
fn genai_rate_limit_false_for_chat_response_with_unrelated_metadata() {
    let body = json!({
        "message": "Bad request",
        "metadata": { "error_type": "invalid_request" }
    });
    let err: GenaiError = GenaiError::ChatResponse {
        model_iden: ModelIden::new(AdapterKind::OpenAI, "some-model"),
        body,
    };
    assert!(!is_rate_limit_error(&err));
}

#[test]
fn genai_rate_limit_false_for_chat_response_empty_body() {
    let body = json!({});
    let err: GenaiError = GenaiError::ChatResponse {
        model_iden: ModelIden::new(AdapterKind::OpenAI, "some-model"),
        body,
    };
    assert!(!is_rate_limit_error(&err));
}

fn usage_span(
    start: usize,
    end: usize,
    prompt: Option<i32>,
    response: Option<i32>,
) -> TokenUsageSpan {
    TokenUsageSpan {
        start,
        end,
        prompt,
        response,
    }
}

fn ledger_with_spans(spans: Vec<TokenUsageSpan>) -> TokenUsageLedger {
    TokenUsageLedger {
        spans,
        ..TokenUsageLedger::default()
    }
}

#[test]
fn tail_start_searches_back_over_the_token_window() {
    // spans as recorded per turn: prompt range followed by the assistant reply
    let ledger = ledger_with_spans(vec![
        usage_span(0, 1, Some(10_000), None),
        usage_span(1, 2, None, Some(500)),
        usage_span(1, 3, Some(10_000), None),
        usage_span(3, 4, None, Some(500)),
        usage_span(3, 5, Some(10_000), None),
        usage_span(5, 6, None, Some(500)),
    ]);

    assert_eq!(ledger.tail_start(20_000), Some(1));
}

#[test]
fn tail_start_none_when_history_fits_the_window() {
    let ledger = ledger_with_spans(vec![
        usage_span(0, 1, Some(10_000), None),
        usage_span(1, 2, None, Some(500)),
    ]);

    assert_eq!(ledger.tail_start(24_000), None);
    assert_eq!(TokenUsageLedger::default().tail_start(24_000), None);
}

#[test]
fn split_point_searches_back_from_the_end() {
    let messages = vec![
        ChatMessage::user("one"),
        ChatMessage::assistant("two"),
        ChatMessage::user("three"),
        ChatMessage::assistant("four"),
        ChatMessage::user("five"),
        ChatMessage::assistant("six"),
        ChatMessage::user("seven"),
    ];
    let ledger = ledger_with_spans(vec![
        usage_span(0, 2, Some(5_000), None),
        usage_span(2, 4, Some(5_000), None),
        usage_span(4, 7, Some(25_000), None),
    ]);

    // the last three messages already cover the searched back tokens
    assert_eq!(ledger.tail_start(24_000), Some(4));
    assert_eq!(Compact::split_point(&messages, &ledger), Some(4));
}

#[test]
fn split_point_keeps_tool_call_with_its_result() {
    let messages = vec![
        ChatMessage::user("one"),
        ChatMessage::assistant("two"),
        ChatMessage::assistant(MessageContent::from_tool_calls(vec![ToolCall {
            call_id: "call-1".to_string(),
            fn_name: "read_file".to_string(),
            fn_arguments: json!({"file_path": "src/lib.rs"}),
            thought_signatures: None,
        }])),
        ChatMessage::tool(MessageContent::from_tool_responses(vec![
            ToolResponse::new("call-1", "file contents"),
        ])),
        ChatMessage::user("three"),
        ChatMessage::assistant("four"),
    ];
    // the search back lands on the tool response message
    let ledger = ledger_with_spans(vec![
        usage_span(0, 3, Some(5_000), None),
        usage_span(3, 6, Some(25_000), None),
    ]);

    assert_eq!(ledger.tail_start(24_000), Some(3));
    assert_eq!(Compact::split_point(&messages, &ledger), Some(2));
}

#[test]
fn split_point_none_without_history_to_compact() {
    let messages = vec![ChatMessage::user("one"), ChatMessage::assistant("two")];
    let ledger = ledger_with_spans(vec![usage_span(0, 2, Some(30_000), None)]);

    assert_eq!(Compact::split_point(&messages, &ledger), None);
}

#[test]
fn compacted_summary_replays_as_compaction_events() {
    let messages = vec![
        ChatMessage::user("one"),
        ChatMessage::assistant("two"),
        ChatMessage::user("three"),
    ];
    let compacted = Compact::compact_history(&messages, 2, "the summary".to_string());

    let events = replay_events(compacted);

    assert_eq!(
        events,
        vec![
            HarnessEvent::CompactStart { start_time: 0 },
            HarnessEvent::CompactSummary(
                "The earlier part of this conversation was compacted into the following \
                 summary to free up context. Continue from where it leaves off.\n\nthe summary"
                    .to_string()
            ),
            HarnessEvent::UserPrompt("three".to_string()),
        ]
    );
}

#[test]
fn compacted_history_is_summary_followed_by_tail() {
    let messages = vec![
        ChatMessage::user("one"),
        ChatMessage::assistant("two"),
        ChatMessage::user("three"),
        ChatMessage::assistant("four"),
    ];

    let compacted = Compact::compact_history(&messages, 2, "the summary".to_string());

    // only the summary replaces the compacted prefix
    assert_eq!(compacted.len(), 3);
    assert_eq!(compacted[0].role, genai::chat::ChatRole::User);
    assert_eq!(
        compacted[0].content.first_text(),
        Some(
            "The earlier part of this conversation was compacted into the following summary to free up context. Continue from where it leaves off.\n\nthe summary"
        )
    );
    assert_eq!(compacted[1].content.first_text(), Some("three"));
    assert_eq!(compacted[2].content.first_text(), Some("four"));
}

#[test]
fn ledger_compact_drops_removed_spans_and_shifts_the_tail() {
    let mut ledger = ledger_with_spans(vec![
        usage_span(0, 2, Some(100), None),
        usage_span(2, 3, None, Some(20)),
        usage_span(2, 5, Some(120), None),
        usage_span(5, 6, None, Some(50)),
    ]);

    ledger.compact(4, None);

    assert_eq!(
        ledger.spans,
        vec![
            // the prompt span [2, 5) straddles the removed messages, so it is
            // clipped to the kept tail instead of reaching over the summary
            usage_span(1, 2, Some(120), None),
            usage_span(2, 3, None, Some(50)),
        ]
    );
    assert_eq!(ledger.last_prompt_tokens, None);
    assert_eq!(ledger.last_prompt_index, None);
}

#[test]
fn compact_records_the_summary_as_the_first_message() {
    let mut ledger = ledger_with_spans(vec![
        usage_span(0, 2, Some(100), None),
        usage_span(2, 5, Some(120), None),
        usage_span(5, 6, None, Some(50)),
    ]);

    ledger.compact(4, Some(30));

    // the summary cost is the span of the first message and sits in front so
    // the spans stay ordered by message index
    assert_eq!(
        ledger.spans,
        vec![
            usage_span(0, 1, None, Some(30)),
            usage_span(1, 2, Some(120), None),
            usage_span(2, 3, None, Some(50)),
        ]
    );
    // the span ordering still lets the tail search back over the history
    assert_eq!(ledger.tail_start(50), Some(2));
    // and the summary cost is replayed as the usage of the first message
    let summary_usage: Vec<_> = ledger.spans_after_message(0).collect();
    assert_eq!(summary_usage.len(), 1);
    assert_eq!(summary_usage[0].response, Some(30));
}

#[test]
fn compact_without_usage_records_no_summary_span() {
    let mut ledger = ledger_with_spans(vec![usage_span(2, 5, Some(120), None)]);

    ledger.compact(4, None);

    assert_eq!(ledger.spans, vec![usage_span(1, 2, Some(120), None)]);
}

#[test]
fn compact_reestimates_context_from_the_surviving_spans() {
    // a 128k model whose last request reported a 119k context
    let mut ledger = ledger_with_spans(vec![
        usage_span(0, 2, Some(70_000), None),
        usage_span(2, 3, None, Some(1_000)),
        usage_span(2, 4, Some(25_000), None),
        usage_span(4, 5, None, Some(1_000)),
        usage_span(4, 6, Some(20_000), None),
        usage_span(6, 7, None, Some(2_000)),
    ]);
    ledger.usage.context_tokens = Some(119_000);

    // the summary request itself: 95k in, 3k out
    ledger.record_side_usage(
        &Usage {
            prompt_tokens: Some(95_000),
            completion_tokens: Some(3_000),
            ..Usage::default()
        },
        None,
    );
    ledger.compact(2, Some(3_000));

    // neither the stale pre-compaction total nor the summary request's own
    // usage may survive as the context size, or the trigger would fire again
    // and re-summarize the fresh summary. The estimate covers the summary
    // plus the kept tail only.
    assert_eq!(ledger.total_tokens(), Some(52_000));
}

#[test]
fn side_usage_counts_towards_the_session_without_spans() {
    let mut ledger = ledger_with_spans(vec![usage_span(0, 2, Some(100), None)]);

    ledger.record_side_usage(
        &Usage {
            prompt_tokens: Some(900),
            completion_tokens: Some(40),
            ..Usage::default()
        },
        None,
    );

    // the compaction request has no place in the history, so no spans
    assert_eq!(ledger.spans, vec![usage_span(0, 2, Some(100), None)]);
    assert_eq!(ledger.usage.session_input, Some(900));
    assert_eq!(ledger.usage.session_output, Some(40));
    assert_eq!(ledger.usage.session_turns, Some(1));
    // and it says nothing about the size of the current context
    assert_eq!(ledger.usage.context_tokens, None);
    assert_eq!(ledger.usage.context_turns, None);
    assert_eq!(ledger.last_prompt_tokens, None);
    assert_eq!(ledger.last_prompt_index, None);
}

#[test]
fn side_usage_accumulates_cost() {
    let mut ledger = TokenUsageLedger::default();
    let pricing = TokenPrice {
        price_in: Some(0.001),
        price_read: Some(0.0001),
        price_out: Some(0.002),
    };

    ledger.record_side_usage(
        &Usage {
            prompt_tokens: Some(1_000),
            completion_tokens: Some(100),
            ..Usage::default()
        },
        Some(&pricing),
    );

    let usage = ledger.usage();
    assert_eq!(usage.cost_in, Some(1.0));
    assert_eq!(usage.cost_out, Some(0.2));
    assert_eq!(usage.total_cost(), Some(1.2));
}

#[test]
fn harness_record_usage_emits_event_and_updates_session_info() {
    let mut harness = Harness::new_without_session(basic_agent()).unwrap();
    let mut events = Vec::new();

    harness.record_usage(
        &Usage {
            prompt_tokens: Some(500),
            completion_tokens: Some(50),
            ..Usage::default()
        },
        &mut |event| {
            events.push(event);
            None
        },
    );

    // the side request's usage is not the context size, so the event carries
    // no total and leaves the context display alone
    assert_eq!(
        events,
        vec![HarnessEvent::TokenUsage {
            prompt: Some(500),
            response: Some(50),
            total: None,
            cost: None,
        }]
    );
    assert_eq!(
        harness.session_info().lock().unwrap().usage.session_input,
        Some(500)
    );
    assert_eq!(harness.snapshot().total_cost, None);
}
