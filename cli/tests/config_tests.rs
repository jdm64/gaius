use gaius::{
    auth::file::OAuthFile,
    config::{Config, ProviderConfig},
    dirs::Dirs,
};
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn with_temp_home(test: impl FnOnce(PathBuf)) {
    let lock = ENV_LOCK.get_or_init(|| Mutex::new(()));
    // Recover from poisoning: a panic inside one test must not turn every
    // other test into a `PoisonError`, which would hide the real failure.
    let _guard = lock.lock().unwrap_or_else(|err| err.into_inner());
    let home = std::env::temp_dir().join(format!(
        "gaius-config-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home).unwrap();
    let previous_home = std::env::var_os("HOME");
    unsafe {
        std::env::set_var("HOME", &home);
    }

    test(home.clone());

    if let Some(previous_home) = previous_home {
        unsafe {
            std::env::set_var("HOME", previous_home);
        }
    } else {
        unsafe {
            std::env::remove_var("HOME");
        }
    }
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn add_provider_persists_provider_to_config_file() {
    with_temp_home(|_| {
        let mut config = Config::new();
        config
            .add_provider(ProviderConfig {
                name: "local".to_string(),
                kind: "openai".to_string(),
                url: "http://localhost:8080/v1".to_string(),
                key: "test-key".to_string(),
            })
            .unwrap();

        let contents = std::fs::read_to_string(Dirs::config_file().unwrap()).unwrap();
        assert!(contents.contains("[[provider]]"));
        assert!(contents.contains("name = \"local\""));
        assert!(contents.contains("url = \"http://localhost:8080/v1\""));
    });
}

#[test]
fn add_provider_rejects_duplicate_names() {
    with_temp_home(|_| {
        let mut config = Config::new();
        let provider = ProviderConfig {
            name: "local".to_string(),
            kind: "openai".to_string(),
            url: "http://localhost:8080/v1".to_string(),
            key: "test-key".to_string(),
        };
        config.add_provider(provider.clone()).unwrap();

        let err = config.add_provider(provider).unwrap_err().to_string();
        assert!(err.contains("already exists"));
    });
}

#[test]
fn building_a_path_does_not_create_it() {
    with_temp_home(|home| {
        // Locating a file should not leave directories behind: a read of the
        // model cache, or of the login, must not create a tree as a side
        // effect of asking where it lives.
        for path in [
            Dirs::data_dir().unwrap(),
            Dirs::cache_dir().unwrap(),
            Dirs::config_dir().unwrap(),
            Dirs::sessions_dir().unwrap(),
        ] {
            assert!(!path.exists(), "{} was created", path.display());
        }

        assert!(!Dirs::auth_file("codex").unwrap().exists());
        assert!(!Dirs::session_file("abc").unwrap().exists());
        assert!(!Dirs::models_cache().unwrap().exists());
        assert!(!Dirs::models_recent().unwrap().exists());
        assert!(!Dirs::config_file().unwrap().exists());

        // The home itself is all a lookup is allowed to need.
        assert!(home.is_dir());
    });
}

#[test]
fn writing_a_token_creates_the_directory_it_needs() {
    with_temp_home(|_| {
        let path = Dirs::auth_file("codex").unwrap();
        assert!(!path.exists());

        let token = OAuthFile {
            id_token: "id".to_string(),
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            account_id: Some("acct".to_string()),
            expires: 1_767_225_600,
        };
        token.save("codex").unwrap();

        assert!(path.is_file());
    });
}

#[test]
fn a_provider_name_cannot_escape_the_data_directory() {
    with_temp_home(|_| {
        assert!(Dirs::auth_file("codex").is_ok());
        assert!(Dirs::auth_file("../../etc/passwd").is_err());
        assert!(Dirs::auth_file("").is_err());
    });
}

// Lives beside `with_temp_home` because it needs the same lock: `Dirs` reads
// `HOME` from the environment, and the lock is per test binary, so a second
// binary mutating `HOME` on its own would race this one.
#[test]
fn saving_a_session_creates_the_sessions_directory() {
    use gaius::session::Session;
    use gaius::token_usage::TokenUsageLedger;
    use genai::chat::ChatRequest;

    with_temp_home(|_| {
        let sessions = Dirs::sessions_dir().unwrap();
        assert!(!sessions.exists(), "{} was created", sessions.display());

        let session = Session::new_named("a-session".to_string()).unwrap();
        session
            .save(&ChatRequest::new(vec![]), &TokenUsageLedger::default())
            .expect("save a session into a home that has no sessions directory");

        assert!(sessions.is_dir());
        let saved = Session::list();
        assert_eq!(
            vec!["a-session".to_string()],
            saved
                .iter()
                .map(|s| s.id.clone().unwrap())
                .collect::<Vec<_>>()
        );
        assert!(session.load().is_ok());
    });
}
