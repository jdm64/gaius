use gaius::tui::{TuiApp, wrapped_line_count};
use genai::chat::{ChatMessage, ChatRequest, ContentPart, MessageContent, ToolCall, ToolResponse};

#[test]
fn counts_wrapped_history_lines() {
    let lines = vec![
        ratatui::text::Line::from("12345"),
        ratatui::text::Line::from("123456"),
    ];

    assert_eq!(wrapped_line_count(&lines, 5), 3);
    assert_eq!(wrapped_line_count(&lines, 0), 11);
}

#[test]
fn loads_chat_request_into_tui_history() {
    let request = ChatRequest {
        system: Some("be useful".to_string()),
        messages: vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant(MessageContent::from_parts(vec![
                ContentPart::Text("hi".to_string()),
                ContentPart::ToolCall(ToolCall {
                    call_id: "123".to_string(),
                    fn_name: "weather".to_string(),
                    fn_arguments: serde_json::json!({"city": "Atlanta"}),
                    thought_signatures: None,
                }),
            ])),
            ChatMessage {
                role: genai::chat::ChatRole::Tool,
                content: MessageContent::from_parts(vec![ContentPart::ToolResponse(
                    ToolResponse::new("123", "sunny"),
                )]),
                options: None,
            },
        ],
        tools: None,
    };

    let mut app = TuiApp::new();
    app.load_history(&request);

    assert_eq!(app.messages.len(), 5);
    assert_eq!(app.messages[0].role, gaius::tui::MessageRole::Agent);
    assert_eq!(app.messages[0].text, "system: be useful");
    assert_eq!(app.messages[1].role, gaius::tui::MessageRole::User);
    assert_eq!(app.messages[1].text, "hello");
    assert_eq!(app.messages[2].role, gaius::tui::MessageRole::Agent);
    assert_eq!(app.messages[2].text, "hi");
    assert_eq!(app.messages[3].role, gaius::tui::MessageRole::ToolCall);
    assert_eq!(app.messages[3].text, "weather ({\"city\":\"Atlanta\"})");
    assert_eq!(app.messages[4].role, gaius::tui::MessageRole::ToolCall);
    assert_eq!(app.messages[4].text, "123 => sunny");
}
