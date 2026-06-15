use gaius::{
    agents::AgentDefinition,
    harness::{Harness, HarnessEvent},
    token_usage::{TokenUsageLedger, TokenUsageSpan},
};
use genai::chat::{ChatMessage, ContentPart, Usage};

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
    );

    let spans = ledger.record(
        4,
        4,
        &Usage {
            prompt_tokens: Some(210),
            completion_tokens: Some(50),
            ..Usage::default()
        },
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
