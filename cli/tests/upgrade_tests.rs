use gaius::upgrade::{Release, Upgrader};
use std::{fs, path::PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct TempPath(PathBuf);

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn test_dir() -> TempPath {
    let path = std::env::temp_dir().join(format!("gaius-upgrade-test-{}", uuid::Uuid::now_v7()));
    fs::create_dir(&path).unwrap();
    TempPath(path)
}

async fn server(responses: Vec<String>) -> &'static str {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let base = url.clone();
    tokio::spawn(async move {
        for template in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            let (headers, body) = template.split_once("\r\n\r\n").unwrap();
            let status = headers
                .lines()
                .next()
                .unwrap()
                .strip_prefix("HTTP/1.1 ")
                .unwrap();
            let response = response(status, &body.replace("{base}", &base));
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    Box::leak(url.into_boxed_str())
}

fn response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[test]
fn version_and_asset_selection() {
    let release: Release = serde_json::from_str(r#"{"tag_name":"v1.10.0","assets":[{"name":"gaius-linux-x86_64","browser_download_url":"https://example.com/binary"}]}"#).unwrap();
    assert!(release.is_newer_than("v1.9.0").unwrap());
    assert!(!release.is_newer_than("v1.10.0").unwrap());
    assert!(!release.is_newer_than("v2.0.0").unwrap());
    assert!(release.is_newer_than("v1.10.0-rc.1").unwrap());
    assert!(release.is_newer_than("not-a-version").is_err());
    assert_eq!(
        release.download_url("linux-x86_64").unwrap(),
        "https://example.com/binary"
    );
    assert!(release.download_url("macos-aarch64").is_err());
}

#[tokio::test]
async fn upgrades_atomically_and_preserves_permissions() {
    let dir = test_dir();
    let executable = dir.0.join("gaius");
    fs::write(&executable, "old binary").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o751)).unwrap();
    }
    let release = r#"{"tag_name":"v1.1.0","assets":[{"name":"gaius-test-platform","browser_download_url":"{base}/binary"}]}"#;
    let url = server(vec![
        response("200 OK", release),
        response("200 OK", "new binary"),
    ])
    .await;

    let upgrader = Upgrader::new_from(url, "v1.0.0", "test-platform", executable.clone()).unwrap();
    upgrader.run().await.unwrap();

    assert_eq!(fs::read(&executable).unwrap(), b"new binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&executable).unwrap().permissions().mode() & 0o777,
            0o751
        );
    }
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[tokio::test]
async fn no_update_does_not_touch_executable() {
    let url = server(vec![response(
        "200 OK",
        r#"{"tag_name":"v1.0.0","assets":[]}"#,
    )])
    .await;
    let dir = test_dir();
    let nonexistent = dir.0.join("gaius");
    Upgrader::new_from(url, "v1.0.0", "test", nonexistent.clone())
        .unwrap()
        .run()
        .await
        .unwrap();
    assert!(!nonexistent.exists());
}

#[tokio::test]
async fn empty_download_preserves_original() {
    let dir = test_dir();
    let executable = dir.0.join("gaius");
    fs::write(&executable, "old binary").unwrap();
    let release = r#"{"tag_name":"v2.0.0","assets":[{"name":"gaius-test","browser_download_url":"{base}/binary"}]}"#;
    let url = server(vec![response("200 OK", release), response("200 OK", "")]).await;
    assert!(
        Upgrader::new_from(url, "v1.0.0", "test", executable.clone())
            .unwrap()
            .run()
            .await
            .is_err()
    );
    assert_eq!(fs::read(&executable).unwrap(), b"old binary");
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[tokio::test]
async fn missing_asset_preserves_original_and_cleans_staging() {
    let dir = test_dir();
    let executable = dir.0.join("gaius");
    fs::write(&executable, "old binary").unwrap();
    let url = server(vec![response(
        "200 OK",
        r#"{"tag_name":"v2.0.0","assets":[]}"#,
    )])
    .await;

    assert!(
        Upgrader::new_from(url, "v1.0.0", "test", executable.clone())
            .unwrap()
            .run()
            .await
            .is_err()
    );
    assert_eq!(fs::read(&executable).unwrap(), b"old binary");
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[tokio::test]
async fn missing_executable_does_not_leave_staging_file() {
    let dir = test_dir();
    let executable = dir.0.join("gaius");
    let url = server(vec![response(
        "200 OK",
        r#"{"tag_name":"v2.0.0","assets":[]}"#,
    )])
    .await;

    assert!(
        Upgrader::new_from(url, "v1.0.0", "test", executable)
            .unwrap()
            .run()
            .await
            .is_err()
    );
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
}

#[tokio::test]
async fn cancelled_download_cleans_staging() {
    let dir = test_dir();
    let executable = dir.0.join("gaius");
    fs::write(&executable, "old binary").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let release = format!(
        r#"{{"tag_name":"v2.0.0","assets":[{{"name":"gaius-test","browser_download_url":"{url}/binary"}}]}}"#
    );
    let (started, received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        socket.read(&mut request).await.unwrap();
        socket
            .write_all(response("200 OK", &release).as_bytes())
            .await
            .unwrap();
        let (_socket, _) = listener.accept().await.unwrap();
        started.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    let upgrader = Upgrader::new_from(
        Box::leak(url.into_boxed_str()),
        "v1.0.0",
        "test",
        executable.clone(),
    )
    .unwrap();
    {
        let run = upgrader.run();
        tokio::pin!(run);
        tokio::select! {
            result = &mut run => panic!("upgrade unexpectedly finished: {result:?}"),
            result = received => result.unwrap(),
        }
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    }
    server.abort();
    assert_eq!(fs::read(&executable).unwrap(), b"old binary");
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[tokio::test]
async fn failed_download_preserves_original_and_cleans_staging() {
    let dir = test_dir();
    let executable = dir.0.join("gaius");
    fs::write(&executable, "old binary").unwrap();
    let release = r#"{"tag_name":"v2.0.0","assets":[{"name":"gaius-test","browser_download_url":"{base}/binary"}]}"#;
    let url = server(vec![
        response("200 OK", release),
        response("404 Not Found", "missing"),
    ])
    .await;
    assert!(
        Upgrader::new_from(url, "v1.0.0", "test", executable.clone())
            .unwrap()
            .run()
            .await
            .is_err()
    );
    assert_eq!(fs::read(&executable).unwrap(), b"old binary");
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
}
