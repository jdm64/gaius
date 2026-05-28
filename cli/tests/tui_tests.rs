use gaius::tui::{TuiApp, TuiMessage, wrapped_line_count};

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
fn build_tui_app_messages_with_all_variants() {
    let mut app = TuiApp::new();
    app.push_message(TuiMessage::SystemMessage("system: be useful".to_string()));
    app.push_message(TuiMessage::UserPrompt("hello".to_string()));
    app.push_message(TuiMessage::AgentMessage("hi".to_string()));
    app.push_message(TuiMessage::ToolCall {
        name: "weather".to_string(),
        arguments: serde_json::json!({"city":"Atlanta"}).to_string(),
        result: String::new(),
        error: false,
    });
    app.push_message(TuiMessage::ToolCall {
        name: "123".to_string(),
        arguments: String::new(),
        result: "sunny".to_string(),
        error: false,
    });

    assert_eq!(app.messages.len(), 5);
    match &app.messages[0] {
        TuiMessage::SystemMessage(t) => assert_eq!(t, "system: be useful"),
        _ => panic!("expected SystemMessage"),
    }
    match &app.messages[1] {
        TuiMessage::UserPrompt(t) => assert_eq!(t, "hello"),
        _ => panic!("expected UserPrompt"),
    }
    match &app.messages[2] {
        TuiMessage::AgentMessage(t) => assert_eq!(t, "hi"),
        _ => panic!("expected AgentMessage"),
    }
    match &app.messages[3] {
        TuiMessage::ToolCall {
            name,
            arguments,
            result,
            error,
        } => {
            assert_eq!(name, "weather");
            assert_eq!(arguments, r#"{"city":"Atlanta"}"#);
            assert!(result.is_empty());
            assert!(!error);
        }
        _ => panic!("expected ToolCall"),
    }
    match &app.messages[4] {
        TuiMessage::ToolCall { name, result, .. } => {
            assert_eq!(name, "123");
            assert_eq!(result, "sunny");
        }
        _ => panic!("expected ToolCall"),
    }
}
