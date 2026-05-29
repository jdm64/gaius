use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gaius::{
    commands::{Commands, wrap},
    config::Config,
    input::{InputMode, PickList, ProviderInfoRow},
    tui::TuiApp,
};

#[test]
fn wrap_behaves_correctly() {
    // Basic wrapping within bounds
    assert_eq!(wrap(0, 5), 0);
    assert_eq!(wrap(2, 5), 2);
    assert_eq!(wrap(4, 5), 4);

    // Wrapping around at boundaries
    assert_eq!(wrap(5, 5), 0);
    assert_eq!(wrap(6, 5), 1);
    assert_eq!(wrap(9, 5), 4);

    // Negative indices wrap to end
    assert_eq!(wrap(-1, 5), 4);
    assert_eq!(wrap(-2, 5), 3);
    assert_eq!(wrap(-5, 5), 0);
    assert_eq!(wrap(-6, 5), 4);

    // Large numbers wrap correctly
    assert_eq!(wrap(12, 5), 2);
    assert_eq!(wrap(20, 7), 6);
    assert_eq!(wrap(100, 10), 0);

    // Edge case: single element
    assert_eq!(wrap(0, 1), 0);
    assert_eq!(wrap(10, 1), 0);
    assert_eq!(wrap(-1, 1), 0);
}

#[tokio::test]
async fn add_provider_mode_moves_between_fields_and_preserves_values() {
    let config = Config::new();
    let mut app = TuiApp::new(config);
    app.input = "local".to_string();
    app.input_cursor = app.input.chars().count();
    let rows = vec![
        ProviderInfoRow::Name(String::new()),
        ProviderInfoRow::Url(String::new()),
        ProviderInfoRow::Kind("openai".to_string()),
        ProviderInfoRow::Key(String::new()),
    ];
    let picker = PickList::all(rows);

    let mode = Commands::handle_add_provider_mode(
        &mut app,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        picker,
    )
    .await;

    match mode {
        InputMode::AddProvider { picker } => {
            assert_eq!(picker.selected, 1);
            assert_eq!(picker.rows[0].value(), "local");
            assert_eq!(picker.rows[1].value(), "");
            assert_eq!(picker.rows[2].value(), "openai");
            assert_eq!(app.input, "");
            assert_eq!(app.input_cursor, 0);
        }
        _ => panic!("expected add provider mode"),
    }
}

#[tokio::test]
async fn add_provider_mode_cancel_returns_to_models() {
    let config = Config::new();
    let mut app = TuiApp::new(config);
    let rows = vec![
        ProviderInfoRow::Name("local".to_string()),
        ProviderInfoRow::Url(String::new()),
        ProviderInfoRow::Kind("openai".to_string()),
        ProviderInfoRow::Key(String::new()),
    ];
    let picker = PickList::all(rows);

    let mode = Commands::handle_add_provider_mode(
        &mut app,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        picker,
    )
    .await;

    assert!(matches!(mode, InputMode::Models { .. }));
    assert_eq!(app.status, "Add provider cancelled");
}
