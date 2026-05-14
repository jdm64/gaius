use gaius::input::Input;
use gaius::tui::TuiApp;

#[test]
fn edits_input_at_cursor() {
    let mut app = TuiApp::new();
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
    let mut app = TuiApp::new();
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
    let mut app = TuiApp::new();
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
    let mut app = TuiApp::new();
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
    let mut app = TuiApp::new();
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
    let mut app = TuiApp::new();

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
