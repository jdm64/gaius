use gaius::skills::SkillRepo;
use gaius::tools::{ToolEngine, ToolName, ToolResult};
use serde_json::json;
use std::sync::{Mutex, OnceLock};

#[test]
fn tool_names_are_single_source_of_truth() {
    for tool in ToolName::ALL {
        assert_eq!(ToolName::from_name(tool.as_str()), Some(tool));
    }
    assert_eq!(ToolName::from_name("unknown"), None);

    let mut names = ToolName::ALL
        .iter()
        .map(|tool| tool.as_str())
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), ToolName::ALL.len());
}

#[test]
fn detects_too_loose_glob_patterns() {
    for pattern in ["", "*", "**", "**/*", "*/**"] {
        assert!(ToolEngine::is_glob_too_loose(pattern), "{pattern:?}");
    }
}

#[test]
fn allows_glob_patterns_with_literal_characters() {
    for pattern in ["*.rs", "src/**/*.toml", "foo/**/bar"] {
        assert!(!ToolEngine::is_glob_too_loose(pattern), "{pattern:?}");
    }
}

#[tokio::test]
async fn plan_tool_returns_plan_text() {
    let result = ToolEngine::new(SkillRepo::default())
        .execute(
            "plan",
            &json!({
                "content": "# Implement feature\n\nBackground information"
            }),
        )
        .await;

    match result {
        ToolResult::Text(text) => {
            assert_eq!("Plan created", text);
        }
        _ => panic!("Expected ToolResult::Text"),
    }
}

#[tokio::test]
async fn plan_tool_renders_arbitrary_fields() {
    let result = ToolEngine::new(SkillRepo::default())
        .execute(
            "plan",
            &json!({
                "content": "# Refactor auth\n\nRisks and considerations"
            }),
        )
        .await;

    match result {
        ToolResult::Text(text) => {
            assert_eq!("Plan created", text);
        }
        _ => panic!("Expected ToolResult::Text"),
    }
}

#[tokio::test]
async fn plan_tool_requires_content() {
    let result = ToolEngine::new(SkillRepo::default())
        .execute("plan", &json!({ "goal": "Refactor auth" }))
        .await;

    match result {
        ToolResult::Error(text) => {
            assert_eq!(text, "Error: Missing content");
        }
        other => panic!("Expected ToolResult::Error, got: {:?}", other),
    }
}

#[tokio::test]
async fn edit_file_returns_compact_diff_view() {
    let _guard = cwd_lock().lock().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    let dir = std::env::temp_dir().join(format!("gaius-edit-file-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_current_dir(&dir).unwrap();
    std::fs::write("sample.txt", "one\ntwo\nthree\nfour\nfive\n").unwrap();

    let result = ToolEngine::new(SkillRepo::default())
        .execute(
            "edit_file",
            &json!({
                "file_path": "sample.txt",
                "old_string": "three\n",
                "new_string": "THREE\n"
            }),
        )
        .await;

    std::env::set_current_dir(original_dir).unwrap();
    let updated = std::fs::read_to_string(dir.join("sample.txt")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(updated, "one\ntwo\nTHREE\nfour\nfive\n");
    match result {
        ToolResult::FileEdit { message, diff } => {
            assert_eq!(message, "File edited successfully");
            assert_eq!(diff.file_path, "sample.txt");
            assert_eq!(diff.hunks.len(), 1);
            let hunk = &diff.hunks[0];
            assert_eq!(hunk.old_start, 1);
            assert_eq!(hunk.new_start, 1);
            assert!(hunk.lines.iter().any(|line| line.text == "three"));
            assert!(hunk.lines.iter().any(|line| line.text == "THREE"));
            assert!(!format!("{diff:?}").contains("one\\ntwo\\nthree\\nfour\\nfive"));
        }
        other => panic!("Expected ToolResult::FileEdit, got: {:?}", other),
    }
}

fn cwd_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[tokio::test]
async fn skill_tool_returns_body_for_existing_skill() {
    use gaius::skills::Skill;

    let mut repo = SkillRepo::default();
    repo.insert(Skill {
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        body: "Skill instructions here".to_string(),
    });

    let result = ToolEngine::new(repo)
        .execute(
            "skill",
            &json!({
                "name": "test-skill"
            }),
        )
        .await;

    match result {
        ToolResult::Text(text) => {
            assert_eq!(text, "Skill instructions here");
        }
        other => panic!("Expected ToolResult::Text, got: {:?}", other),
    }
}

#[tokio::test]
async fn skill_tool_returns_error_for_missing_skill() {
    let result = ToolEngine::new(SkillRepo::default())
        .execute(
            "skill",
            &json!({
                "name": "nonexistent"
            }),
        )
        .await;

    match result {
        ToolResult::Error(text) => {
            assert!(text.contains("not found"), "Error: {}", text);
        }
        other => panic!("Expected ToolResult::Error, got: {:?}", other),
    }
}

#[tokio::test]
async fn skill_tool_requires_name() {
    let result = ToolEngine::new(SkillRepo::default())
        .execute("skill", &json!({}))
        .await;

    match result {
        ToolResult::Error(text) => {
            assert_eq!(text, "Missing name");
        }
        other => panic!("Expected ToolResult::Error, got: {:?}", other),
    }
}

/// Spawn a minimal HTTP server that responds once with the given status line,
/// content type, and body. Returns the URL the `webfetch` tool should fetch.
fn spawn_test_server(
    status_line: &'static str,
    content_type: &'static str,
    body: &'static str,
) -> String {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let mut received = Vec::new();
            // Read until we have received the full request headers.
            loop {
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        received.extend_from_slice(&buf[..n]);
                        if received.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}/")
}

#[tokio::test]
async fn webfetch_returns_cleaned_html_text() {
    let url = spawn_test_server(
        "200 OK",
        "text/html; charset=utf-8",
        "<html><head><title>ignored</title></head>\
         <body><h1>Hello</h1><p>World &amp; friends</p>\
         <script>alert('leak')</script>\
         <style>.x{display:none}</style></body></html>",
    );
    let result = ToolEngine::new(SkillRepo::default())
        .execute("webfetch", &json!({ "url": url }))
        .await;

    match result {
        ToolResult::Text(text) => {
            assert!(text.contains("Hello"), "missing heading: {text}");
            assert!(
                text.contains("World & friends"),
                "entity not decoded / missing text: {text}"
            );
            assert!(!text.contains('<'), "tags not stripped: {text}");
            assert!(!text.contains("alert"), "script content leaked: {text}");
            assert!(
                !text.contains("display:none"),
                "style content leaked: {text}"
            );
        }
        other => panic!("Expected ToolResult::Text, got: {other:?}"),
    }
}

#[tokio::test]
async fn webfetch_returns_raw_text_for_non_html() {
    let url = spawn_test_server("200 OK", "application/json", "{\"hello\":\"world\"}");
    let result = ToolEngine::new(SkillRepo::default())
        .execute("webfetch", &json!({ "url": url }))
        .await;

    match result {
        ToolResult::Text(text) => {
            assert_eq!(text, "{\"hello\":\"world\"}");
        }
        other => panic!("Expected ToolResult::Text, got: {other:?}"),
    }
}

#[tokio::test]
async fn webfetch_returns_error_on_non_success() {
    let url = spawn_test_server("404 Not Found", "text/plain", "not found");
    let result = ToolEngine::new(SkillRepo::default())
        .execute("webfetch", &json!({ "url": url }))
        .await;

    match result {
        ToolResult::Error(text) => {
            assert!(text.contains("404"), "expected status in error: {text}");
        }
        other => panic!("Expected ToolResult::Error, got: {other:?}"),
    }
}

#[tokio::test]
async fn webfetch_requires_url() {
    let result = ToolEngine::new(SkillRepo::default())
        .execute("webfetch", &json!({}))
        .await;

    match result {
        ToolResult::Error(text) => {
            assert_eq!(text, "Missing url");
        }
        other => panic!("Expected ToolResult::Error, got: {other:?}"),
    }
}

#[tokio::test]
async fn webfetch_rejects_unsupported_scheme() {
    let result = ToolEngine::new(SkillRepo::default())
        .execute("webfetch", &json!({ "url": "file:///etc/passwd" }))
        .await;

    match result {
        ToolResult::Error(text) => {
            assert!(text.contains("scheme"), "expected scheme error: {text}");
        }
        other => panic!("Expected ToolResult::Error, got: {other:?}"),
    }
}

#[test]
fn test_build_tools_without_plan() {
    let skill_repo = SkillRepo::default();
    let engine = ToolEngine::new(skill_repo);

    let tools = engine.build_tools_without_plan();

    // Should have all tools except Plan
    assert_eq!(tools.len(), ToolName::ALL.len() - 1);

    // Verify that Plan is not included but all other tools are
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        !tool_names.contains(&"plan"),
        "build_tools_without_plan should not include the Plan tool"
    );

    for tool_name in &ToolName::ALL {
        if *tool_name == ToolName::Plan {
            assert!(
                !tool_names.contains(&tool_name.as_str()),
                "Plan tool should be absent"
            );
        } else {
            assert!(
                tool_names.contains(&tool_name.as_str()),
                "Tool {} should be present",
                tool_name.as_str()
            );
        }
    }
}
