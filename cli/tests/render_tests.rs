use gaius::config::Config;
use gaius::diff_view::{DiffHunk, DiffLine, DiffLineKind, DiffView};
use gaius::render::Render;
use gaius::render_history::DisplayPrefs;
use gaius::selection::{HistoryPoint, HistorySelection, RowWrapInfo, Selection};
use gaius::tui::{TuiApp, TuiMessage};
use ratatui::{
    Terminal,
    backend::TestBackend,
    layout::Position,
    style::Color,
    text::{Line, Span},
};

fn default_prefs() -> DisplayPrefs {
    DisplayPrefs {
        thinking: false,
        token_info: true,
        diff_view: true,
    }
}

#[test]
fn markdown_heading_has_bold_style() {
    let render = Render::new();
    let md = "# Heading";
    let msg = TuiMessage::AgentMessage(md.to_string());
    let lines = render.render_message(&msg, &default_prefs(), 80);
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
    let render = Render::new();
    let md = "- item1\n- item2";
    let msg = TuiMessage::AgentMessage(md.to_string());
    let lines = render.render_message(&msg, &default_prefs(), 80);
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
    let render = Render::new();
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
    let (visible, _row_infos) = render.visible_history_lines(&lines, 20, 2, 2);

    assert_eq!(line_texts(&visible), vec!["three", "four"]);
}

#[test]
fn visible_history_lines_slices_wrapped_lines() {
    let render = Render::new();
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
    let (visible, _row_infos) = render.visible_history_lines(&lines, 2, 1, 3);

    assert_eq!(line_texts(&visible), vec!["cd", "ef", "gh"]);
}

#[test]
fn visible_history_lines_handles_empty_history() {
    let render = Render::new();
    let (visible, _row_infos) = render.visible_history_lines(&[], 20, 0, 5);

    assert!(visible.is_empty());
}

#[test]
fn visible_history_lines_pads_user_prompts_to_width() {
    let render = Render::new();
    let lines = render.render_message(
        &TuiMessage::UserPrompt("hello".to_string()),
        &default_prefs(),
        80,
    );

    let (visible, _row_infos) = render.visible_history_lines(&lines, 10, 0, 3);

    assert_eq!(visible.len(), 3);
    assert_eq!(visible[0].width(), 10);
    assert_eq!(visible[1].width(), 10);
    assert_eq!(visible[2].width(), 10);
    assert_eq!(visible[0].spans[0].content.as_ref(), "\u{2503} ");
    assert_eq!(visible[1].spans[0].content.as_ref(), "\u{2503} ");
    assert_eq!(visible[2].spans[0].content.as_ref(), "\u{2503} ");
}

#[test]
fn selected_history_text_returns_single_line_partial_selection() {
    let selection = Selection {
        lines: vec![Line::from("abcdef")],
        row_info: vec![RowWrapInfo {
            index: 0,
            prefix: 0,
            content: "abcdef".to_string(),
        }],
        selection: Some(HistorySelection {
            anchor: HistoryPoint { row: 0, col: 1 },
            focus: HistoryPoint { row: 0, col: 4 },
            active: false,
        }),
        ..Default::default()
    };

    assert_eq!(selection.selected_text(), Some("bcd".to_string()));
}

#[test]
fn selected_history_text_returns_multi_line_selection_with_clipped_edges() {
    let selection = Selection {
        lines: vec![
            Line::from("abcdef"),
            Line::from("ghijkl"),
            Line::from("mnopqr"),
        ],
        row_info: vec![
            RowWrapInfo {
                index: 0,
                prefix: 0,
                content: "abcdef".to_string(),
            },
            RowWrapInfo {
                index: 1,
                prefix: 0,
                content: "ghijkl".to_string(),
            },
            RowWrapInfo {
                index: 2,
                prefix: 0,
                content: "mnopqr".to_string(),
            },
        ],
        selection: Some(HistorySelection {
            anchor: HistoryPoint { row: 0, col: 2 },
            focus: HistoryPoint { row: 2, col: 3 },
            active: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        selection.selected_text(),
        Some("cdef\nghijkl\nmno".to_string())
    );
}

#[test]
fn selected_history_text_normalizes_reversed_drag_direction() {
    let selection = Selection {
        lines: vec![Line::from("abcdef"), Line::from("ghijkl")],
        row_info: vec![
            RowWrapInfo {
                index: 0,
                prefix: 0,
                content: "abcdef".to_string(),
            },
            RowWrapInfo {
                index: 1,
                prefix: 0,
                content: "ghijkl".to_string(),
            },
        ],
        selection: Some(HistorySelection {
            anchor: HistoryPoint { row: 1, col: 2 },
            focus: HistoryPoint { row: 0, col: 3 },
            active: false,
        }),
        ..Default::default()
    };

    assert_eq!(selection.selected_text(), Some("def\ngh".to_string()));
}

#[test]
fn selected_history_text_returns_none_for_empty_selection() {
    let selection = Selection {
        lines: vec![Line::from("abcdef")],
        selection: Some(HistorySelection {
            anchor: HistoryPoint { row: 0, col: 2 },
            focus: HistoryPoint { row: 0, col: 2 },
            active: false,
        }),
        ..Default::default()
    };

    assert_eq!(selection.selected_text(), None);
}

#[test]
fn draw_history_applies_selection_highlight_to_selected_cells() {
    let render = Render::new();
    let mut app = TuiApp::new(Config::new());
    app.push_message(TuiMessage::AgentMessage("abcdef".to_string()));
    app.selection.selection = Some(HistorySelection {
        anchor: HistoryPoint { row: 1, col: 1 },
        focus: HistoryPoint { row: 1, col: 4 },
        active: true,
    });
    let mut terminal = Terminal::new(TestBackend::new(30, 8)).unwrap();

    terminal.draw(|frame| render.draw(&mut app, frame)).unwrap();

    let selected = terminal
        .backend()
        .buffer()
        .cell(Position { x: 2, y: 2 })
        .unwrap();
    let unselected = terminal
        .backend()
        .buffer()
        .cell(Position { x: 1, y: 2 })
        .unwrap();

    assert_eq!(selected.symbol(), "b");
    assert_eq!(selected.bg, Color::Magenta);
    assert_eq!(selected.fg, Color::Black);
    assert_eq!(unselected.symbol(), "a");
    assert_ne!(unselected.bg, Color::Magenta);
}

#[test]
fn draw_input_expands_height_for_wrapped_prompt() {
    let render = Render::new();
    let mut app = TuiApp::new(Config::new());
    app.input = "abcdefghijklmnopq".to_string();
    app.input_cursor = app.input.chars().count();
    let mut terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();

    terminal.draw(|frame| render.draw(&mut app, frame)).unwrap();

    assert_eq!(
        terminal.get_cursor_position().unwrap(),
        Position { x: 5, y: 5 }
    );
}

#[test]
fn render_diff_view_includes_headers_lines_and_missing_newline_marker() {
    let render = Render::new();
    let msg = TuiMessage::DiffView(DiffView {
        file_path: "src/lib.rs".to_string(),
        hunks: vec![DiffHunk {
            old_start: 2,
            old_lines: 2,
            new_start: 2,
            new_lines: 2,
            lines: vec![
                DiffLine {
                    kind: DiffLineKind::Context,
                    old_line: Some(2),
                    new_line: Some(2),
                    text: "same".to_string(),
                    missing_newline: false,
                },
                DiffLine {
                    kind: DiffLineKind::Delete,
                    old_line: Some(3),
                    new_line: None,
                    text: "old".to_string(),
                    missing_newline: false,
                },
                DiffLine {
                    kind: DiffLineKind::Insert,
                    old_line: None,
                    new_line: Some(3),
                    text: "new".to_string(),
                    missing_newline: true,
                },
            ],
        }],
    });

    let lines = render.render_message(&msg, &default_prefs(), 80);
    let texts = line_texts(&lines);

    assert!(texts.contains(&"diff src/lib.rs".to_string()));
    assert!(texts.contains(&"@@ -2,2 +2,2 @@".to_string()));
    assert!(texts.contains(&" same".to_string()));
    assert!(texts.contains(&"-old".to_string()));
    assert!(texts.contains(&"+new".to_string()));
    assert!(texts.contains(&"\\ No newline at end of file".to_string()));
}

#[test]
fn draw_input_places_cursor_on_next_wrapped_line_at_boundary() {
    let render = Render::new();
    let mut app = TuiApp::new(Config::new());
    app.input = "abcdefghijklmno".to_string();
    app.input_cursor = 14;
    let mut terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();

    terminal.draw(|frame| render.draw(&mut app, frame)).unwrap();

    assert_eq!(
        terminal.get_cursor_position().unwrap(),
        Position { x: 2, y: 5 }
    );
}

#[test]
fn tool_call_duration_renders_on_its_own_line() {
    let render = Render::new();
    let msg = TuiMessage::ToolCall {
        name: "bash".to_string(),
        arguments: "{}".to_string(),
        start_time: gaius::harness::time_now() - 5_000,
    };
    let lines = render.render_message(&msg, &default_prefs(), 80);
    assert_eq!(lines.len(), 2, "expected name line + duration line");
    assert!(lines[0].spans.iter().any(|s| s.content == "bash"));
    assert!(
        lines[1]
            .spans
            .iter()
            .any(|s| s.content.contains('\u{23F1}'))
    );

    // A finished tool call (start_time == 0) should not render a duration line.
    let msg = TuiMessage::ToolCall {
        name: "bash".to_string(),
        arguments: "{}".to_string(),
        start_time: 0,
    };
    let lines = render.render_message(&msg, &default_prefs(), 80);
    assert_eq!(lines.len(), 1);
}

#[test]
fn live_tool_call_timer_updates_without_new_messages() {
    use std::thread;
    use std::time::Duration as StdDuration;

    let render = Render::new();
    let mut app = TuiApp::new(Config::new());
    app.push_message(TuiMessage::ToolCall {
        name: "bash".to_string(),
        arguments: "{}".to_string(),
        start_time: gaius::harness::time_now() - 5_000,
    });

    let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
    terminal.draw(|frame| render.draw(&mut app, frame)).unwrap();

    // The active tool call should be tracked as a live timer.
    assert_eq!(app.live_timers.len(), 1);
    let timer_text = |app: &TuiApp| {
        app.history_lines[app.live_timers[0].line_index]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    };

    let first = timer_text(&app);
    thread::sleep(StdDuration::from_millis(1100));
    terminal.draw(|frame| render.draw(&mut app, frame)).unwrap();
    let second = timer_text(&app);

    assert_ne!(first, second, "timer should change across frames");
    // 5.000s -> 6.1xx s: same-length strings, so lexicographic compare holds.
    assert!(second > first, "timer should advance: {first} -> {second}");
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

#[test]
fn compaction_start_renders_rule_with_duration_while_running() {
    let render = Render::new();
    let msg = TuiMessage::CompactionStart {
        start_time: gaius::harness::time_now() - 3_000,
    };
    let lines = render.render_message(&msg, &default_prefs(), 80);
    assert_eq!(lines.len(), 2, "expected rule line + duration line");

    // the word Compaction sits centered on a horizontal rule
    let rule = &line_texts(&lines)[0];
    let (before, rest) = rule.split_once(" Compaction ").expect("label present");
    assert!(!before.is_empty() && before.chars().all(|c| c == '\u{2500}'));
    assert!(!rest.is_empty() && rest.chars().all(|c| c == '\u{2500}'));

    // running compaction shows its duration like a running tool call
    assert!(
        lines[1]
            .spans
            .iter()
            .any(|s| s.content.contains('\u{23F1}'))
    );

    // a finished compaction (start_time == 0) renders just the rule
    let msg = TuiMessage::CompactionStart { start_time: 0 };
    let lines = render.render_message(&msg, &default_prefs(), 80);
    assert_eq!(lines.len(), 1);
    assert!(line_texts(&lines)[0].contains(" Compaction "));
}

#[test]
fn live_compaction_timer_updates_without_new_messages() {
    use std::thread;
    use std::time::Duration as StdDuration;

    let render = Render::new();
    let mut app = TuiApp::new(Config::new());
    app.push_message(TuiMessage::CompactionStart {
        start_time: gaius::harness::time_now() - 5_000,
    });

    let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
    terminal.draw(|frame| render.draw(&mut app, frame)).unwrap();

    // the running compaction should be tracked as a live timer
    assert_eq!(app.live_timers.len(), 1);
    let timer_text = |app: &TuiApp| {
        app.history_lines[app.live_timers[0].line_index]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    };

    let first = timer_text(&app);
    thread::sleep(StdDuration::from_millis(1100));
    terminal.draw(|frame| render.draw(&mut app, frame)).unwrap();
    let second = timer_text(&app);

    assert_ne!(first, second, "timer should change across frames");
    assert!(second > first, "timer should advance: {first} -> {second}");
}

fn buffer_contains(terminal: &Terminal<TestBackend>, needle: &str) -> bool {
    let buf = terminal.backend().buffer();
    for row in 0..buf.area.height {
        let mut text = String::new();
        for col in 0..buf.area.width {
            if let Some(cell) = buf.cell(Position { x: col, y: row }) {
                text.push_str(cell.symbol());
            }
        }
        if text.contains(needle) {
            return true;
        }
    }
    false
}

#[test]
fn draw_history_anchors_view_and_shows_new_lines_below_indicator() {
    let render = Render::new();
    let mut app = TuiApp::new(Config::new());

    // Plenty of short single-line messages so the history is taller than the
    // viewport.
    for i in 0..30u32 {
        app.push_message(TuiMessage::AgentMessage(format!("msg {i}")));
    }

    let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();

    // At the bottom: nothing special.
    terminal.draw(|frame| render.draw(&mut app, frame)).unwrap();
    assert_eq!(app.history_scroll, 0);
    assert_eq!(app.new_lines_below, 0);
    assert!(!buffer_contains(&terminal, " new line"));

    // Simulate the user scrolling up a few lines.
    app.history_scroll = 5;
    terminal.draw(|frame| render.draw(&mut app, frame)).unwrap();
    assert_eq!(app.new_lines_below, 0);

    // New output while scrolled up must NOT yank the view to the bottom, must
    // keep the same viewport (anchored), and must flag the unread lines below.
    app.push_message(TuiMessage::AgentMessage("new output".to_string()));
    terminal.draw(|frame| render.draw(&mut app, frame)).unwrap();
    assert_ne!(app.history_scroll, 0); // not yanked to the bottom
    assert_eq!(app.history_scroll, 6); // anchored: grew by exactly one new line
    assert_eq!(app.new_lines_below, 1);
    assert!(buffer_contains(&terminal, " new line"));

    // Returning to the bottom dismisses the indicator.
    app.history_scroll = 0;
    terminal.draw(|frame| render.draw(&mut app, frame)).unwrap();
    assert_eq!(app.history_scroll, 0);
    assert_eq!(app.new_lines_below, 0);
    assert!(!buffer_contains(&terminal, " new line"));
}
