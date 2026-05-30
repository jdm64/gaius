use gaius::config::{Config, ProviderConfig, config_file};
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn with_temp_home(test: impl FnOnce(PathBuf)) {
    let lock = ENV_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap();
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

        let contents = std::fs::read_to_string(config_file().unwrap()).unwrap();
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
