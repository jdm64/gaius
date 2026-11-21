use gaius::dirs::Dirs;
use std::path::PathBuf;

#[test]
fn sessions_dir_can_be_overridden_and_reset() {
    Dirs::set_sessions_dir(None);
    let default_dir = Dirs::data_dir().unwrap().join("sessions");
    assert_eq!(Dirs::sessions_dir().unwrap(), default_dir);

    let override_dir = PathBuf::from("custom-session-dir");
    Dirs::set_sessions_dir(Some(override_dir.clone()));
    assert_eq!(Dirs::sessions_dir().unwrap(), override_dir);
    assert_eq!(
        Dirs::session_file("session-id").unwrap(),
        PathBuf::from("custom-session-dir/session-id.mpk")
    );

    Dirs::set_sessions_dir(None);
    assert_eq!(Dirs::sessions_dir().unwrap(), default_dir);
}
