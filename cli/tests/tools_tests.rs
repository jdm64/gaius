use gaius::{
    agents::AgentTools,
    skills::{Skill, SkillRepo},
    tool_engine::{ToolEngine, ToolResult},
    tools::ToolName::{self},
};
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
    let result = ToolEngine::new(SkillRepo::default(), 30000, 60)
        .execute(
            "plan",
            &json!({
                "plan": "# Implement feature\n\nBackground information"
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
    let result = ToolEngine::new(SkillRepo::default(), 30000, 60)
        .execute(
            "plan",
            &json!({
                "plan": "# Refactor auth\n\nRisks and considerations"
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
    let result = ToolEngine::new(SkillRepo::default(), 30000, 60)
        .execute("plan", &json!({ "goal": "Refactor auth" }))
        .await;

    match result {
        ToolResult::Failure(text) => {
            assert_eq!(text, "Missing required argument: plan");
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

    let result = ToolEngine::new(SkillRepo::default(), 30000, 60)
        .execute(
            &ToolName::Edit.name(),
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
            assert_eq!(message, "File edited successfully. Made 1 replacements");
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

#[tokio::test]
async fn glob_returns_relative_paths() {
    let _guard = cwd_lock().lock().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    let dir = std::env::temp_dir().join(format!("gaius-glob-path-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("nested")).unwrap();
    std::fs::write(dir.join("nested/sample.txt"), "content").unwrap();
    std::env::set_current_dir(&dir).unwrap();

    let result = ToolEngine::new(SkillRepo::default(), 30000, 60)
        .execute("glob", &json!({ "query": "*.txt", "path": "nested" }))
        .await;

    std::env::set_current_dir(original_dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        ToolResult::Text(text) => assert_eq!(text, "nested/sample.txt"),
        other => panic!("Expected relative glob result, got: {other:?}"),
    }
}

#[tokio::test]
async fn grep_returns_relative_paths() {
    let _guard = cwd_lock().lock().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    let dir = std::env::temp_dir().join(format!("gaius-grep-path-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("nested")).unwrap();
    std::fs::write(dir.join("nested/sample.txt"), "match this\n").unwrap();
    std::env::set_current_dir(&dir).unwrap();

    let result = ToolEngine::new(SkillRepo::default(), 30000, 60)
        .execute(
            "grep",
            &json!({ "query": "match", "path": "nested/sample.txt" }),
        )
        .await;

    std::env::set_current_dir(original_dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        ToolResult::Text(text) => assert_eq!(text, "nested/sample.txt:1: match this"),
        other => panic!("Expected relative grep result, got: {other:?}"),
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

    let result = ToolEngine::new(repo, 30000, 60)
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
    let result = ToolEngine::new(SkillRepo::default(), 30000, 60)
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
    let result = ToolEngine::new(SkillRepo::default(), 30000, 60)
        .execute("skill", &json!({}))
        .await;

    match result {
        ToolResult::Failure(text) => {
            assert_eq!(text, "Missing required argument: name");
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
    let result = ToolEngine::new(SkillRepo::default(), 30000, 60)
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
    let result = ToolEngine::new(SkillRepo::default(), 30000, 60)
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
    let result = ToolEngine::new(SkillRepo::default(), 30000, 60)
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
    let result = ToolEngine::new(SkillRepo::default(), 30000, 60)
        .execute("webfetch", &json!({}))
        .await;

    match result {
        ToolResult::Failure(text) => {
            assert_eq!(text, "Missing required argument: url");
        }
        other => panic!("Expected ToolResult::Error, got: {other:?}"),
    }
}

#[tokio::test]
async fn webfetch_rejects_unsupported_scheme() {
    let result = ToolEngine::new(SkillRepo::default(), 30000, 60)
        .execute("webfetch", &json!({ "url": "file:///etc/passwd" }))
        .await;

    match result {
        ToolResult::Failure(text) => {
            assert!(text.contains("scheme"), "expected scheme error: {text}");
        }
        other => panic!("Expected ToolResult::Error, got: {other:?}"),
    }
}

#[test]
fn build_tools_applies_agent_include_and_exclude_filters() {
    let engine = ToolEngine::new(SkillRepo::default(), 30000, 60);
    let filters = AgentTools {
        include: vec!["read".into(), "bash".into(), "grep".into()],
        exclude: vec!["bash".into()],
    };

    let tools = engine.build_tools(true, Some(&filters));
    let tool_names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();

    assert_eq!(tool_names, ["read", "grep", "plan"]);
}

#[test]
fn test_build_tools_without_plan() {
    let engine = ToolEngine::new(SkillRepo::default(), 30000, 60);
    let tools = engine.build_tools(false, None);

    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    for tool_name in &ToolName::ALL {
        if *tool_name == ToolName::Plan || *tool_name == ToolName::Skill {
            assert!(
                !tool_names.contains(&tool_name.as_str()),
                "Tool {} should be absent",
                tool_name.as_str()
            );
        } else {
            assert!(
                tool_names.contains(&tool_name.as_str()),
                "Tool {} should be present",
                tool_name.as_str()
            );
        }
    }

    let mut skill_repo = SkillRepo::default();
    skill_repo.insert(Skill {
        name: "test-skill".into(),
        description: "description".into(),
        body: "body".into(),
    });
    let engine = ToolEngine::new(skill_repo, 30000, 60);
    let tools = engine.build_tools(true, None);
    assert_eq!(tools.len(), ToolName::ALL.len());
}
