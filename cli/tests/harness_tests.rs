use gaius::harness::{Harness, HarnessEvent};
use genai::chat::{ChatMessage, ContentPart};

fn replay_events(messages: Vec<ChatMessage>) -> Vec<HarnessEvent> {
    let mut events = Vec::new();
    Harness::replay_messages(&messages, |event| events.push(event));
    events
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
