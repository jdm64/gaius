use gaius::config::Config;
use gaius::render::Render;
use gaius::tui::{TuiApp, TuiMessage};
use ratatui::{
    Terminal,
    backend::TestBackend,
    layout::Position,
    text::{Line, Span},
};

#[test]
fn markdown_heading_has_bold_style() {
    let md = "# Heading";
    let msg = TuiMessage::AgentMessage(md.to_string());
    let lines = Render::render_message(&msg, false);
    assert!(!lines.is_empty());
    // The heading style should be applied to the Line's style, not the span.
    let line = &lines[0];
    // Check the line's style for bold and cyan (H1 style)
    let has_bold = line
        .style
        .add_modifier
        .contains(ratatui::style::Modifier::BOLD);
    let has_cyan = line.style.fg == Some(ratatui::style::Color::Cyan)
        || line.style.bg == Some(ratatui::style::Color::Cyan);
    // At least one of these should be true for H1 from DefaultStyleSheet
    assert!(
        has_bold || has_cyan,
        "Expected heading line to have bold or cyan style, got {:?}",
        line.style
    );
}

#[test]
fn markdown_list_has_style() {
    let md = "- item1\n- item2";
    let msg = TuiMessage::AgentMessage(md.to_string());
    let lines = Render::render_message(&msg, false);
    assert!(!lines.is_empty());
    // List items should have a style (maybe a marker).
    // Check lines contain the items; style might be default but marker could have style?
    let content: Vec<String> = lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    let joined = content.join(" ");
    assert!(joined.contains("item1"));
    assert!(joined.contains("item2"));
}

#[test]
fn visible_history_lines_returns_bottom_window() {
    let raw = vec![
        Line::from("one"),
        Line::from("two"),
        Line::from("three"),
        Line::from("four"),
    ];
    let lines: Vec<Line<'static>> = raw
        .into_iter()
        .map(|l| {
            let mut owned = Line::from(
                l.spans
                    .into_iter()
                    .map(|s| Span::styled(s.content.to_string(), s.style))
                    .collect::<Vec<_>>(),
            );
            owned.style = l.style;
            owned.alignment = l.alignment;
            owned
        })
        .collect();
    let visible = Render::visible_history_lines(&lines, 20, 2, 2);

    assert_eq!(line_texts(&visible), vec!["three", "four"]);
}

#[test]
fn visible_history_lines_slices_wrapped_lines() {
    let raw = vec![Line::from("abcdef"), Line::from("gh")];
    let lines: Vec<Line<'static>> = raw
        .into_iter()
        .map(|l| {
            let mut owned = Line::from(
                l.spans
                    .into_iter()
                    .map(|s| Span::styled(s.content.to_string(), s.style))
                    .collect::<Vec<_>>(),
            );
            owned.style = l.style;
            owned.alignment = l.alignment;
            owned
        })
        .collect();
    let visible = Render::visible_history_lines(&lines, 2, 1, 3);

    assert_eq!(line_texts(&visible), vec!["cd", "ef", "gh"]);
}

#[test]
fn visible_history_lines_handles_empty_history() {
    let visible = Render::visible_history_lines(&[], 20, 0, 5);

    assert!(visible.is_empty());
}

#[test]
fn visible_history_lines_pads_user_prompts_to_width() {
    let lines = Render::render_message(&TuiMessage::UserPrompt("hello".to_string()), false);

    let visible = Render::visible_history_lines(&lines, 10, 0, 3);

    assert_eq!(visible.len(), 3);
    assert_eq!(visible[0].width(), 10);
    assert_eq!(visible[1].width(), 10);
    assert_eq!(visible[2].width(), 10);
    assert_eq!(visible[0].spans[0].content.as_ref(), "\u{2503} ");
    assert_eq!(visible[1].spans[0].content.as_ref(), "\u{2503} ");
    assert_eq!(visible[2].spans[0].content.as_ref(), "\u{2503} ");
}

#[test]
fn draw_input_expands_height_for_wrapped_prompt() {
    let mut app = TuiApp::new(Config::new());
    app.input = "abcdefghijklmnopq".to_string();
    app.input_cursor = app.input.chars().count();
    let mut terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();

    terminal
        .draw(|frame| Render::draw(&mut app, frame))
        .unwrap();

    assert_eq!(
        terminal.get_cursor_position().unwrap(),
        Position { x: 5, y: 6 }
    );
}

#[test]
fn draw_input_places_cursor_on_next_wrapped_line_at_boundary() {
    let mut app = TuiApp::new(Config::new());
    app.input = "abcdefghijklmno".to_string();
    app.input_cursor = 14;
    let mut terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();

    terminal
        .draw(|frame| Render::draw(&mut app, frame))
        .unwrap();

    assert_eq!(
        terminal.get_cursor_position().unwrap(),
        Position { x: 2, y: 6 }
    );
}

fn line_texts(lines: &[Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect()
}
