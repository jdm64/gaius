use gaius::config::Config;
use gaius::input::{Input, PickList};
use gaius::tui::TuiApp;

#[test]
fn edits_input_at_cursor() {
    let mut app = TuiApp::new(Config::new());
    Input::insert_input_char(&mut app, 'a');
    Input::insert_input_char(&mut app, 'c');

    Input::move_input_cursor_left(&mut app);
    Input::insert_input_char(&mut app, 'b');

    assert_eq!(app.input, "abc");
    assert_eq!(app.input_cursor, 2);

    Input::delete_input_char_before_cursor(&mut app);

    assert_eq!(app.input, "ac");
    assert_eq!(app.input_cursor, 1);

    Input::delete_input_char_at_cursor(&mut app);

    assert_eq!(app.input, "a");
    assert_eq!(app.input_cursor, 1);
}

#[test]
fn moves_input_cursor_home_and_end() {
    let mut app = TuiApp::new(Config::new());
    for ch in "prompt".chars() {
        Input::insert_input_char(&mut app, ch);
    }

    Input::move_input_cursor_home(&mut app);
    assert_eq!(app.input_cursor, 0);

    Input::move_input_cursor_end(&mut app);
    assert_eq!(app.input_cursor, 6);
}

#[test]
fn edits_multibyte_input_at_cursor() {
    let mut app = TuiApp::new(Config::new());
    for ch in "aéc".chars() {
        Input::insert_input_char(&mut app, ch);
    }

    Input::move_input_cursor_left(&mut app);
    Input::insert_input_char(&mut app, 'b');
    Input::move_input_cursor_left(&mut app);
    Input::delete_input_char_before_cursor(&mut app);

    assert_eq!(app.input, "abc");
    assert_eq!(app.input_cursor, 1);
}

#[test]
fn deletes_input_to_start_and_end() {
    let mut app = TuiApp::new(Config::new());
    for ch in "abcdef".chars() {
        Input::insert_input_char(&mut app, ch);
    }

    Input::move_input_cursor_left(&mut app);
    Input::move_input_cursor_left(&mut app);
    Input::delete_input_to_start(&mut app);

    assert_eq!(app.input, "ef");
    assert_eq!(app.input_cursor, 0);

    Input::move_input_cursor_end(&mut app);
    Input::move_input_cursor_left(&mut app);
    Input::delete_input_to_end(&mut app);

    assert_eq!(app.input, "e");
    assert_eq!(app.input_cursor, 1);
}

#[test]
fn deletes_multibyte_input_to_start_and_end() {
    let mut app = TuiApp::new(Config::new());
    for ch in "aé文z".chars() {
        Input::insert_input_char(&mut app, ch);
    }

    Input::move_input_cursor_left(&mut app);
    Input::move_input_cursor_left(&mut app);
    Input::delete_input_to_start(&mut app);

    assert_eq!(app.input, "文z");
    assert_eq!(app.input_cursor, 0);

    Input::move_input_cursor_right(&mut app);
    Input::delete_input_to_end(&mut app);

    assert_eq!(app.input, "文");
    assert_eq!(app.input_cursor, 1);
}

#[test]
fn scrolls_history_with_saturating_offsets() {
    let mut app = TuiApp::new(Config::new());

    assert_eq!(app.history_scroll, 0);

    Input::scroll_history_up(&mut app, 5);
    assert_eq!(app.history_scroll, 5);

    Input::scroll_history_down(&mut app, 2);
    assert_eq!(app.history_scroll, 3);

    Input::scroll_history_down(&mut app, 10);
    assert_eq!(app.history_scroll, 0);

    Input::scroll_history_up(&mut app, 4);
    Input::reset_history_scroll(&mut app);
    assert_eq!(app.history_scroll, 0);
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
    assert_eq!(Input::get_file_query(empty_query, empty_query.len()), None);

    assert_eq!(
        Input::replace_file_query("myfile.txt", empty_query, empty_query.len()),
        empty_query
    );

    assert_eq!(Input::get_file_query("@", 1), Some("".to_string()));

    assert_eq!(
        Input::replace_file_query("myfile.txt", "@", 1),
        "myfile.txt".to_string()
    );

    let input = "@one.txt foo @two.txt bar @three.txt";
    assert_eq!(Input::get_file_query(input, 8), Some("one.txt".to_string()));

    assert_eq!(
        Input::replace_file_query("myfile.txt", input, 8),
        "myfile.txt foo @two.txt bar @three.txt".to_string()
    );

    assert_eq!(
        Input::get_file_query(input, 21),
        Some("two.txt".to_string())
    );

    assert_eq!(
        Input::replace_file_query("myfile.txt", input, 21),
        "@one.txt foo myfile.txt bar @three.txt".to_string()
    );

    assert_eq!(
        Input::get_file_query(input, input.len()),
        Some("three.txt".to_string())
    );

    assert_eq!(
        Input::replace_file_query("myfile.txt", input, input.len()),
        "@one.txt foo @two.txt bar myfile.txt".to_string()
    );

    assert_eq!(
        Input::get_file_query("text @foo text", 8),
        Some("fo".to_string())
    );

    assert_eq!(
        Input::replace_file_query("bar.foo", "text @foo text", 8),
        "text bar.foo text".to_string()
    );

    assert_eq!(
        Input::get_file_query("text @é文 text", 8),
        Some("é文".to_string())
    );

    assert_eq!(
        Input::replace_file_query("myfile.txt", "text @é文 text", 8),
        "text myfile.txt text".to_string()
    );
}
