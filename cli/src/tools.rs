/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::diff_view::DiffView;
use crate::skills::SkillRepo;
use genai::chat::*;
use glob::glob;
use regex::Regex;
use serde_json::Value;
use serde_json::json;
use std::io::ErrorKind;
use std::io::Write;
use std::process::Command;
use std::time::Duration;
use url::Url;

#[derive(Debug)]
pub enum ToolResult {
    Error(String),
    Text(String),
    FileEdit { message: String, diff: DiffView },
    Question(String, Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolName {
    ReadFile,
    CreateFile,
    EditFile,
    Bash,
    Glob,
    Grep,
    Question,
    Plan,
    Skill,
    WebFetch,
}

impl ToolName {
    pub const ALL: [ToolName; 10] = [
        ToolName::ReadFile,
        ToolName::CreateFile,
        ToolName::EditFile,
        ToolName::Bash,
        ToolName::Glob,
        ToolName::Grep,
        ToolName::Question,
        ToolName::Plan,
        ToolName::Skill,
        ToolName::WebFetch,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ToolName::ReadFile => "read_file",
            ToolName::CreateFile => "create_file",
            ToolName::EditFile => "edit_file",
            ToolName::Bash => "bash",
            ToolName::Glob => "glob",
            ToolName::Grep => "grep",
            ToolName::Question => "question",
            ToolName::Plan => "plan",
            ToolName::Skill => "skill",
            ToolName::WebFetch => "webfetch",
        }
    }

    pub fn from_name(name: &str) -> Option<ToolName> {
        ToolName::ALL
            .iter()
            .copied()
            .find(|tool| tool.as_str() == name)
    }

    pub fn display_fields(self) -> &'static [&'static str] {
        match self {
            ToolName::ReadFile => &["file_path", "start_line", "max_lines"],
            ToolName::CreateFile => &["file_path"],
            ToolName::EditFile => &["file_path"],
            ToolName::Bash => &["command"],
            ToolName::Glob => &["path", "pattern"],
            ToolName::Grep => &["path", "include", "pattern"],
            ToolName::Question => &["title"],
            ToolName::Plan => &[],
            ToolName::Skill => &["name"],
            ToolName::WebFetch => &["url"],
        }
    }

    fn genai_tool(self) -> Tool {
        match self {
            ToolName::ReadFile => Tool::new(self.as_str())
                .with_description("Read the contents of a file")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "The path to the file to read"
                        },
                        "start_line": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Optional one-based line number to start reading from"
                        },
                        "max_lines": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Optional maximum number of lines to read"
                        }
                    },
                    "required": ["file_path"]
                })),
            ToolName::EditFile => Tool::new(self.as_str())
                .with_description("Modify an existing file by replacing exactly one string match")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "The path to the file to edit"
                        },
                        "old_string": {
                            "type": "string",
                            "description": "The exact string to find and replace"
                        },
                        "new_string": {
                            "type": "string",
                            "description": "The string to replace the match with"
                        }
                    },
                    "required": ["file_path", "old_string", "new_string"]
                })),
            ToolName::CreateFile => Tool::new(self.as_str())
                .with_description("Create a new file with the provided contents")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "The path to the file to create"
                        },
                        "contents": {
                            "type": "string",
                            "description": "The contents to write to the new file"
                        }
                    },
                    "required": ["file_path", "contents"]
                })),
            ToolName::Bash => Tool::new(self.as_str())
                .with_description("Execute a bash command")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "The bash command to execute"
                        }
                    },
                    "required": ["command"]
                })),
            ToolName::Glob => Tool::new(self.as_str())
                .with_description("Find files matching a glob pattern")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Glob pattern to match files (e.g., '**/*.rs', 'src/**/*.toml')"
                        },
                        "path": {
                            "type": "string",
                            "description": "Optional directory to search in (defaults to current directory)"
                        }
                    },
                    "required": ["pattern"]
                })),
            ToolName::Grep => Tool::new(self.as_str())
                .with_description("Search file contents using regex pattern")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Regex pattern to search for"
                        },
                        "path": {
                            "type": "string",
                            "description": "File or directory path to search"
                        },
                        "include": {
                            "type": "string",
                            "description": "Optional glob pattern to filter which files to search (e.g., '*.rs')"
                        },
                        "recursive": {
                            "type": "boolean",
                            "description": "Whether to search recursively in directories (default: true)"
                        }
                    },
                    "required": ["pattern", "path"]
                })),
            ToolName::Skill => Tool::new(self.as_str())
                .with_description("Invoke a skill by name to get its instructions")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Name of skill to invoke"
                        }
                    },
                    "required": ["name"],
                })),
            ToolName::Question => Tool::new(self.as_str())
                .with_description("Ask the user a question with optional choices")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "The question or prompt to show the user"
                        },
                        "options": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional list of choices for the user"
                        }
                    },
                    "required": ["title"]
                })),
            ToolName::Plan => Tool::new(self.as_str())
                .with_description("Create a structured markdown formatted plan")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "The content of the plan"
                        }
                    },
                    "required": ["content"],
                })),
            ToolName::WebFetch => Tool::new(self.as_str())
                .with_description("Fetch a URL and return its content as cleaned Markdown text")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "The fully-qualified http(s) URL to fetch"
                        },
                    },
                    "required": ["url"]
                })),
        }
    }
}

pub struct ToolEngine {
    pub skill_repo: SkillRepo,
}

impl ToolEngine {
    pub fn new(skill_repo: SkillRepo) -> Self {
        Self { skill_repo }
    }

    pub fn build_tools(&self) -> Vec<Tool> {
        ToolName::ALL.iter().map(|tool| tool.genai_tool()).collect()
    }

    pub fn build_tools_without_plan(&self) -> Vec<Tool> {
        ToolName::ALL
            .iter()
            .filter(|tool| **tool != ToolName::Plan)
            .map(|tool| tool.genai_tool())
            .collect()
    }

    pub async fn execute(&self, name: &str, args: &Value) -> ToolResult {
        match ToolName::from_name(name) {
            Some(ToolName::ReadFile) => self.read_file_tool(args),
            Some(ToolName::CreateFile) => self.create_file_tool(args),
            Some(ToolName::EditFile) => self.edit_file_tool(args),
            Some(ToolName::Bash) => self.bash_tool(args),
            Some(ToolName::Glob) => self.glob_tool(args),
            Some(ToolName::Grep) => self.grep_tool(args),
            Some(ToolName::Question) => self.question_tool(args),
            Some(ToolName::Plan) => self.plan_tool(args),
            Some(ToolName::Skill) => self.skill_tool(args),
            Some(ToolName::WebFetch) => Self::webfetch_tool(args).await,
            None => ToolResult::Error(format!("Unknown tool call: {} ({})", name, args)),
        }
    }

    pub fn load_skill(&self, name: &str) -> ToolResult {
        match self.skill_repo.find(name) {
            Ok(body) => ToolResult::Text(body),
            Err(e) => ToolResult::Error(e.to_string()),
        }
    }

    fn skill_tool(&self, args: &Value) -> ToolResult {
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return ToolResult::Error("Missing name".to_string()),
        };

        self.load_skill(name)
    }

    fn read_file_tool(&self, args: &Value) -> ToolResult {
        let file_path_str = match args.get("file_path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::Error("Missing file_path".to_string()),
        };
        let start_line = match args.get("start_line") {
            Some(value) => match value.as_u64() {
                Some(line) if line >= 1 => line as usize,
                _ => {
                    return ToolResult::Error(
                        "start_line must be an integer greater than or equal to 1".to_string(),
                    );
                }
            },
            None => 1,
        };
        let max_lines = match args.get("max_lines") {
            Some(value) => match value.as_u64() {
                Some(lines) => Some(lines as usize),
                _ => {
                    return ToolResult::Error(
                        "max_lines must be a non-negative integer".to_string(),
                    );
                }
            },
            None => None,
        };
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => return ToolResult::Error(format!("Error getting current directory: {}", e)),
        };
        let full_path = cwd.join(file_path_str);
        match std::fs::read_to_string(full_path) {
            Ok(content) => {
                if start_line == 1 && max_lines.is_none() {
                    return ToolResult::Text(content);
                }

                let lines = content.split_inclusive('\n').skip(start_line - 1);
                let result = match max_lines {
                    Some(max_lines) => lines.take(max_lines).collect(),
                    None => lines.collect(),
                };
                ToolResult::Text(result)
            }
            Err(e) => ToolResult::Error(format!("Error reading file: {}", e)),
        }
    }

    fn create_file_tool(&self, args: &Value) -> ToolResult {
        let file_path_str = match args.get("file_path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::Error("Missing file_path".to_string()),
        };
        let contents = match args.get("contents").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::Error("Missing contents".to_string()),
        };
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => return ToolResult::Error(format!("Error getting current directory: {}", e)),
        };
        let full_path = cwd.join(file_path_str);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(full_path)
        {
            Ok(mut file) => match file.write_all(contents.as_bytes()) {
                Ok(()) => ToolResult::Text("File written successfully".to_string()),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => ToolResult::Error(
                    "create_file cannot overwrite an existing file. Use edit_file instead"
                        .to_string(),
                ),
                Err(e) => ToolResult::Error(format!("Error writing file: {}", e)),
            },
            Err(e) => ToolResult::Error(format!("Error creating file: {}", e)),
        }
    }

    fn edit_file_tool(&self, args: &Value) -> ToolResult {
        let file_path_str = match args.get("file_path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::Error("Missing file_path parameter".to_string()),
        };
        let find = match args.get("old_string").and_then(|v| v.as_str()) {
            Some(f) => f,
            None => {
                return ToolResult::Error(
                    "Missing old_string parameter (the exact text to find in the file)".to_string(),
                );
            }
        };
        let replace = match args.get("new_string").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => {
                return ToolResult::Error(
                    "Missing new_string parameter (the replacement text)".to_string(),
                );
            }
        };
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => return ToolResult::Error(format!("Error getting current directory: {}", e)),
        };
        let full_path = cwd.join(file_path_str);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(content) => content,
            Err(e) => return ToolResult::Error(format!("Error reading file: {}", e)),
        };
        let match_count = content.matches(find).count();
        if match_count == 0 {
            return ToolResult::Error(
                "old_string not found in file. Make sure the old_string parameter \
                exactly matches text in the file, including whitespace and indentation."
                    .to_string(),
            );
        }
        if match_count > 1 {
            return ToolResult::Error(format!(
                "old_string matched {} times. The old_string parameter must match exactly once in \
                 the file. Include more surrounding context to make it unique.",
                match_count
            ));
        }

        let updated = content.replace(find, replace);
        let relative_path = full_path
            .strip_prefix(&cwd)
            .unwrap_or(&full_path)
            .to_string_lossy()
            .into_owned();
        let diff = DiffView::from_text(relative_path, &content, &updated);
        match std::fs::write(full_path, updated) {
            Ok(()) => ToolResult::FileEdit {
                message: "File edited successfully".to_string(),
                diff,
            },
            Err(e) => ToolResult::Error(format!("Error writing file: {}", e)),
        }
    }

    fn bash_tool(&self, args: &Value) -> ToolResult {
        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::Error("Missing command".to_string()),
        };
        match Command::new("bash").arg("-c").arg(command).output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let result = format!(
                    "{}\nstdout:\n{}\nstderr:\n{}",
                    output.status, stdout, stderr
                );

                if output.status.success() {
                    ToolResult::Text(result)
                } else {
                    ToolResult::Error(result)
                }
            }
            Err(e) => ToolResult::Error(format!("Error executing command: {}", e)),
        }
    }

    async fn webfetch_tool(args: &Value) -> ToolResult {
        let url_str = match args.get("url").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => return ToolResult::Error("Missing url".to_string()),
        };

        let parsed = match Url::parse(url_str) {
            Ok(u) => u,
            Err(e) => return ToolResult::Error(format!("Invalid url: {e}")),
        };

        match parsed.scheme() {
            "http" | "https" => {}
            other => return ToolResult::Error(format!("Unsupported url scheme: {other}")),
        }

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("gaius-webfetch/0.1")
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolResult::Error(format!("Failed to build http client: {e}")),
        };

        let resp = match client.get(parsed).send().await {
            Ok(r) => r,
            Err(e) => return ToolResult::Error(format!("Request failed: {e}")),
        };

        let status = resp.status();
        if !status.is_success() {
            return ToolResult::Error(format!("Request returned non-success status: {status}"));
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => return ToolResult::Error(format!("Failed to read response body: {e}")),
        };

        let text = match String::from_utf8(bytes.to_vec()) {
            Ok(t) => t,
            Err(e) => return ToolResult::Error(format!("Response body is not valid UTF-8: {e}")),
        };

        const MAX_CHARS: usize = 50_000;
        let truncate = |s: String| -> String {
            if s.chars().count() > MAX_CHARS {
                let end = s
                    .char_indices()
                    .nth(MAX_CHARS)
                    .map(|(i, _)| i)
                    .unwrap_or(s.len());
                format!("[truncated to first {} chars]\n{}", MAX_CHARS, &s[..end])
            } else {
                s
            }
        };

        if content_type.to_ascii_lowercase().contains("text/html") {
            let cleaned = html2md::rewrite_html(&text, false);
            ToolResult::Text(truncate(cleaned))
        } else {
            // Non-HTML responses (JSON, plain text, XML, etc.) are returned as-is.
            ToolResult::Text(truncate(text))
        }
    }

    fn glob_tool(&self, args: &Value) -> ToolResult {
        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::Error("Missing pattern".to_string()),
        };
        if Self::is_glob_too_loose(pattern) {
            return ToolResult::Error(
                "Glob pattern must contain at least one non-wildcard character".to_string(),
            );
        }
        let path_prefix = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let full_pattern = if path_prefix.is_empty() {
            pattern.to_string()
        } else {
            format!("{}/{}", path_prefix, pattern)
        };
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => return ToolResult::Error(format!("Error getting current directory: {}", e)),
        };
        let full_pattern = if full_pattern.starts_with('/') {
            full_pattern
        } else {
            cwd.join(&full_pattern).to_string_lossy().into_owned()
        };
        match glob(&full_pattern) {
            Ok(entries) => {
                let mut results = Vec::new();
                for entry in entries.filter_map(Result::ok) {
                    results.push(entry.to_string_lossy().into_owned());
                }
                ToolResult::Text(results.join("\n"))
            }
            Err(e) => ToolResult::Error(format!("Error in glob pattern: {}", e)),
        }
    }

    fn question_tool(&self, args: &Value) -> ToolResult {
        let title = match args.get("title").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return ToolResult::Error("Missing title".to_string()),
        };
        let mut options: Vec<String> = args
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if !options.iter().any(|o| o.eq_ignore_ascii_case("other")) {
            options.push("Other:".to_string());
        }
        ToolResult::Question(title.to_string(), options)
    }

    fn plan_tool(&self, args: &Value) -> ToolResult {
        if args.get("content").is_none() {
            return ToolResult::Error("Error: Missing content".to_string());
        }

        ToolResult::Text("Plan created".to_string())
    }

    fn grep_tool(&self, args: &Value) -> ToolResult {
        let pattern_str = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::Error("Missing pattern".to_string()),
        };
        let path_str = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::Error("Missing path".to_string()),
        };
        let include_pattern = args.get("include").and_then(|v| v.as_str());
        let recursive = args
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => return ToolResult::Error(format!("Error getting current directory: {}", e)),
        };
        let full_path = if path_str.starts_with('/') {
            std::path::PathBuf::from(path_str)
        } else {
            cwd.join(path_str)
        };
        let regex = match Regex::new(pattern_str) {
            Ok(r) => r,
            Err(e) => return ToolResult::Error(format!("Error compiling regex: {}", e)),
        };
        let mut results = Vec::new();
        if !full_path.exists() {
            return ToolResult::Error(format!("path does not exist: {}", path_str));
        }
        if full_path.is_file() {
            self.grep_file(&full_path, &regex, &mut results);
        } else if recursive {
            self.grep_directory(&full_path, &regex, include_pattern, &mut results);
        } else {
            return ToolResult::Error("path is a directory but recursive is false".to_string());
        }
        ToolResult::Text(results.join("\n"))
    }

    fn grep_file(&self, path: &std::path::Path, regex: &Regex, results: &mut Vec<String>) {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        for (line_num, line) in content.lines().enumerate() {
            if regex.is_match(line) {
                results.push(format!(
                    "{}:{}: {}",
                    path.to_string_lossy(),
                    line_num + 1,
                    line.trim_start()
                ));
            }
        }
    }

    fn grep_directory(
        &self,
        dir: &std::path::Path,
        regex: &Regex,
        include_pattern: Option<&str>,
        results: &mut Vec<String>,
    ) {
        let include_regex = match include_pattern {
            Some(p) => match glob::Pattern::new(p) {
                Ok(pat) => Some(pat),
                Err(_) => return,
            },
            None => None,
        };
        let walker = walkdir::WalkDir::new(dir).into_iter();
        for entry in walker.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_file() {
                if let Some(pat) = &include_regex {
                    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !pat.matches(filename) {
                        continue;
                    }
                }
                self.grep_file(path, regex, results);
            }
        }
    }

    pub fn is_glob_too_loose(pattern: &str) -> bool {
        !pattern.chars().any(|c| c != '*' && c != '/')
    }
}
