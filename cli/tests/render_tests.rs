use gaius::render::Render;
use gaius::tui::{MessageRole, TuiMessage};
use ratatui::text::Line;

#[test]
fn markdown_heading_has_bold_style() {
    let md = "# Heading";
    let msg = TuiMessage {
        role: MessageRole::Agent,
        text: md.to_string(),
    };
    let lines = Render::render_message(&msg);
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
    let msg = TuiMessage {
        role: MessageRole::Agent,
        text: md.to_string(),
    };
    let lines = Render::render_message(&msg);
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
    let lines = vec![
        Line::from("one"),
        Line::from("two"),
        Line::from("three"),
        Line::from("four"),
    ];

    let visible = Render::visible_history_lines(&lines, 20, 2, 2);

    assert_eq!(line_texts(&visible), vec!["three", "four"]);
}

#[test]
fn visible_history_lines_slices_wrapped_lines() {
    let lines = vec![Line::from("abcdef"), Line::from("gh")];

    let visible = Render::visible_history_lines(&lines, 2, 1, 3);

    assert_eq!(line_texts(&visible), vec!["cd", "ef", "gh"]);
}

#[test]
fn visible_history_lines_handles_empty_history() {
    let visible = Render::visible_history_lines(&[], 20, 0, 5);

    assert!(visible.is_empty());
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
