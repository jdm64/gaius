use gaius::config::Config;
use gaius::input::picklist::PickList;
use gaius::input::{PromptEditor, file};
use gaius::tui::TuiApp;

#[test]
fn edits_input_at_cursor() {
    let app = TuiApp::new(Config::new());
    let mut editor = app.editor;
    editor.insert_input_char('a');
    editor.insert_input_char('c');

    editor.move_input_cursor_left();
    editor.insert_input_char('b');

    assert_eq!(editor.input, "abc");
    assert_eq!(editor.cursor, 2);

    editor.delete_input_char_before_cursor();

    assert_eq!(editor.input, "ac");
    assert_eq!(editor.cursor, 1);

    editor.delete_input_char_at_cursor();

    assert_eq!(editor.input, "a");
    assert_eq!(editor.cursor, 1);
}

#[test]
fn moves_input_cursor_home_and_end() {
    let app = TuiApp::new(Config::new());
    let mut editor = app.editor;
    for ch in "prompt".chars() {
        editor.insert_input_char(ch);
    }

    editor.move_input_cursor_home();
    assert_eq!(editor.cursor, 0);

    editor.move_input_cursor_end();
    assert_eq!(editor.cursor, 6);
}

#[test]
fn edits_multibyte_input_at_cursor() {
    let mut app = TuiApp::new(Config::new());
    for ch in "aéc".chars() {
        app.editor.insert_input_char(ch);
    }

    app.editor.move_input_cursor_left();
    app.editor.insert_input_char('b');
    app.editor.move_input_cursor_left();
    app.editor.delete_input_char_before_cursor();

    assert_eq!(app.editor.input, "abc");
    assert_eq!(app.editor.cursor, 1);
}

#[test]
fn deletes_input_to_start_and_end() {
    let mut app = TuiApp::new(Config::new());
    for ch in "abcdef".chars() {
        app.editor.insert_input_char(ch);
    }

    app.editor.move_input_cursor_left();
    app.editor.move_input_cursor_left();
    app.editor.delete_input_to_start();

    assert_eq!(app.editor.input, "ef");
    assert_eq!(app.editor.cursor, 0);

    app.editor.move_input_cursor_end();
    app.editor.move_input_cursor_left();
    app.editor.delete_input_to_end();

    assert_eq!(app.editor.input, "e");
    assert_eq!(app.editor.cursor, 1);
}

#[test]
fn deletes_multibyte_input_to_start_and_end() {
    let mut app = TuiApp::new(Config::new());
    for ch in "aé文z".chars() {
        app.editor.insert_input_char(ch);
    }

    app.editor.move_input_cursor_left();
    app.editor.move_input_cursor_left();
    app.editor.delete_input_to_start();

    assert_eq!(app.editor.input, "文z");
    assert_eq!(app.editor.cursor, 0);

    app.editor.move_input_cursor_right();
    app.editor.delete_input_to_end();

    assert_eq!(app.editor.input, "文");
    assert_eq!(app.editor.cursor, 1);
}

#[test]
fn scrolls_history_with_saturating_offsets() {
    let mut app = TuiApp::new(Config::new());

    assert_eq!(app.history.scroll, 0);

    app.history.scroll_up(5);
    assert_eq!(app.history.scroll, 5);

    app.history.scroll_down(2);
    assert_eq!(app.history.scroll, 3);

    app.history.scroll_down(10);
    assert_eq!(app.history.scroll, 0);

    // Scrolling up sets the offset; only an explicit force returns to the bottom.
    app.history.scroll_up(4);
    assert_eq!(app.history.scroll, 4);
    app.history.scroll_bottom();
    assert_eq!(app.history.scroll, 0);
}

#[test]
fn pick_list_wraps_selection_through_filtered_rows() {
    let mut list = PickList::new(vec!["a", "b", "c"], vec![0, 2]);

    assert_eq!(list.selected_row(), Some(&"a"));

    list.move_up();
    assert_eq!(list.selected, 1);
    assert_eq!(list.selected_row(), Some(&"c"));

    list.move_down();
    assert_eq!(list.selected, 0);
    assert_eq!(list.selected_row(), Some(&"a"));
}

#[test]
fn pick_list_clamps_after_filter_shrinks() {
    let mut list = PickList::new(vec!["a", "b", "c"], vec![0, 1, 2]);
    list.selected = 2;

    list.replace_filter(vec![1]);

    assert_eq!(list.selected, 0);
    assert_eq!(list.selected_row_index(), Some(1));
    assert_eq!(list.selected_row(), Some(&"b"));
}

#[test]
fn pick_list_handles_empty_filters() {
    let mut list = PickList::new(vec!["a", "b"], Vec::new());

    assert!(list.is_empty());
    assert_eq!(list.selected, 0);
    assert_eq!(list.selected_row(), None);

    list.move_down();
    assert_eq!(list.selected, 0);
}

#[test]
fn file_query_get_and_replace() {
    let empty_query = "no file";
    assert_eq!(
        PromptEditor::get_file_query(empty_query, empty_query.len()),
        None
    );

    assert_eq!(
        file::replace_file_query("myfile.txt", empty_query, empty_query.len()),
        empty_query
    );

    assert_eq!(PromptEditor::get_file_query("@", 1), Some("".to_string()));

    assert_eq!(
        file::replace_file_query("myfile.txt", "@", 1),
        "myfile.txt".to_string()
    );

    let input = "@one.txt foo @two.txt bar @three.txt";
    assert_eq!(
        PromptEditor::get_file_query(input, 8),
        Some("one.txt".to_string())
    );

    assert_eq!(
        file::replace_file_query("myfile.txt", input, 8),
        "myfile.txt foo @two.txt bar @three.txt".to_string()
    );

    assert_eq!(
        PromptEditor::get_file_query(input, 21),
        Some("two.txt".to_string())
    );

    assert_eq!(
        file::replace_file_query("myfile.txt", input, 21),
        "@one.txt foo myfile.txt bar @three.txt".to_string()
    );

    assert_eq!(
        PromptEditor::get_file_query(input, input.len()),
        Some("three.txt".to_string())
    );

    assert_eq!(
        file::replace_file_query("myfile.txt", input, input.len()),
        "@one.txt foo @two.txt bar myfile.txt".to_string()
    );

    assert_eq!(
        PromptEditor::get_file_query("text @foo text", 8),
        Some("fo".to_string())
    );

    assert_eq!(
        file::replace_file_query("bar.foo", "text @foo text", 8),
        "text bar.foo text".to_string()
    );

    assert_eq!(
        PromptEditor::get_file_query("text @é文 text", 8),
        Some("é文".to_string())
    );

    assert_eq!(
        file::replace_file_query("myfile.txt", "text @é文 text", 8),
        "text myfile.txt text".to_string()
    );
}
