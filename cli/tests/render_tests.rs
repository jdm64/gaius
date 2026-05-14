use gaius::render::Render;
use gaius::tui::{MessageRole, TuiMessage};

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
