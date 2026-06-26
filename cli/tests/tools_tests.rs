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

#[test]
fn plan_tool_returns_plan_text() {
    let result = ToolEngine {}.execute(
        "plan",
        &json!({
            "content": "# Implement feature\n\nBackground information"
        }),
    );

    match result {
        ToolResult::Text(text) => {
            assert_eq!("Plan created", text);
        }
        _ => panic!("Expected ToolResult::Text"),
    }
}

#[test]
fn plan_tool_renders_arbitrary_fields() {
    let result = ToolEngine {}.execute(
        "plan",
        &json!({
            "content": "# Refactor auth\n\nRisks and considerations"
        }),
    );

    match result {
        ToolResult::Text(text) => {
            assert_eq!("Plan created", text);
        }
        _ => panic!("Expected ToolResult::Text"),
    }
}

#[test]
fn plan_tool_requires_content() {
    let result = ToolEngine {}.execute("plan", &json!({ "goal": "Refactor auth" }));

    match result {
        ToolResult::Error(text) => {
            assert_eq!(text, "Error: Missing content");
        }
        other => panic!("Expected ToolResult::Error, got: {:?}", other),
    }
}

#[test]
fn edit_file_returns_compact_diff_view() {
    let _guard = cwd_lock().lock().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    let dir = std::env::temp_dir().join(format!("gaius-edit-file-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_current_dir(&dir).unwrap();
    std::fs::write("sample.txt", "one\ntwo\nthree\nfour\nfive\n").unwrap();

    let result = ToolEngine {}.execute(
        "edit_file",
        &json!({
            "file_path": "sample.txt",
            "find": "three\n",
            "replace": "THREE\n"
        }),
    );

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
