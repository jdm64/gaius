/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    agents::AgentTools,
    diff_view::DiffView,
    skills::SkillRepo,
    tools::{ArgName, ToolName},
};
use genai::chat::Tool;
use ignore::{Walk, WalkBuilder, overrides::OverrideBuilder};
use regex::Regex;
use serde_json::Value;
use std::{
    cmp::Reverse,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};
use tokio::process::Command;
use url::Url;
use websearch::{SearchOptions, providers::DuckDuckGoProvider, web_search};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ToolStatus {
    Valid = 0,
    Error = 1,
    Failure = 2,
}

impl ToolStatus {
    pub fn from_value(value: u64) -> Self {
        match value {
            1 => Self::Error,
            2 => Self::Failure,
            _ => Self::Valid,
        }
    }
}

#[derive(Debug)]
pub enum ToolResult {
    Error(String),
    Failure(String),
    Text(String),
    FileEdit { message: String, diff: DiffView },
    Question(String, Vec<String>),
}

impl ToolResult {
    pub fn status(&self) -> ToolStatus {
        match self {
            ToolResult::Error(..) => ToolStatus::Error,
            ToolResult::Failure(..) => ToolStatus::Failure,
            _ => ToolStatus::Valid,
        }
    }
}

pub struct ToolEngine {
    pub proj_dir: PathBuf,
    pub skill_repo: SkillRepo,
    max_output: usize,
    max_output_split: usize,
}

impl ToolEngine {
    pub fn new(
        skill_repo: SkillRepo,
        proj_dir: PathBuf,
        max_output: usize,
        max_output_split: usize,
    ) -> Self {
        Self {
            proj_dir,
            skill_repo,
            max_output,
            max_output_split,
        }
    }

    pub fn truncate_output(&self, output: String) -> String {
        let max = self.max_output;
        let count = output.chars().count();
        if count <= max {
            return output;
        }
        let start_count = max.saturating_mul(self.max_output_split.min(100)) / 100;
        let end_count = max.saturating_sub(start_count);
        let start: String = output.chars().take(start_count).collect();
        let end: String = output.chars().skip(count - end_count).collect();
        format!(
            "[Warning: tool output truncated.]\\n{start}\\n[... {} characters truncated ...]\\n{end}",
            count - max
        )
    }

    fn required_arg(arg: &str) -> ToolResult {
        ToolResult::Failure(format!("Missing required argument: {arg}"))
    }

    pub fn build_tools(&self, plan_mode: bool, filters: Option<&AgentTools>) -> Vec<Tool> {
        let has_skills = !self.skill_repo.is_empty();
        ToolName::ALL
            .iter()
            .filter(|tool| {
                match **tool {
                    ToolName::Plan => return plan_mode,
                    ToolName::Skill => return has_skills,
                    _ => {}
                };

                let Some(filters) = filters else {
                    return true;
                };
                let name = tool.as_str();
                (filters.include.is_empty() || filters.include.iter().any(|item| item == name))
                    && !filters.exclude.iter().any(|item| item == name)
            })
            .map(|tool| tool.genai_tool())
            .collect()
    }

    pub async fn execute(&self, name: &str, args: &Value) -> ToolResult {
        match ToolName::from_name(name) {
            Some(ToolName::Read) => self.read_tool(args),
            Some(ToolName::Write) => self.write_tool(args),
            Some(ToolName::Edit) => self.edit_tool(args),
            Some(ToolName::Bash) => self.bash_tool(args).await,
            Some(ToolName::Glob) => self.glob_tool(args),
            Some(ToolName::Grep) => self.grep_tool(args),
            Some(ToolName::Ask) => self.ask_tool(args),
            Some(ToolName::Plan) => self.plan_tool(args),
            Some(ToolName::Skill) => self.skill_tool(args),
            Some(ToolName::WebFetch) => Self::webfetch_tool(args).await,
            Some(ToolName::WebSearch) => Self::websearch_tool(args).await,
            None => ToolResult::Failure(format!("Unknown tool call: {} ({})", name, args)),
        }
    }

    pub fn load_skill(&self, name: &str) -> ToolResult {
        match self.skill_repo.find(name) {
            Ok(body) => ToolResult::Text(body),
            Err(e) => ToolResult::Error(e.to_string()),
        }
    }

    fn skill_tool(&self, args: &Value) -> ToolResult {
        let name = match args.get(ArgName::Name).and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return Self::required_arg(ArgName::Name),
        };

        self.load_skill(name)
    }

    fn read_tool(&self, args: &Value) -> ToolResult {
        let file_path_str = match args.get(ArgName::FilePath).and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return Self::required_arg(ArgName::FilePath),
        };
        let start_line = match args.get(ArgName::StartLine) {
            Some(value) => match value.as_u64() {
                Some(line) if line >= 1 => line as usize,
                _ => {
                    return ToolResult::Failure(
                        "start_line must be an integer greater than or equal to 1".to_string(),
                    );
                }
            },
            None => 1,
        };
        let max_lines = match args.get(ArgName::MaxLines) {
            Some(value) => match value.as_u64() {
                Some(0) => None,
                Some(lines) => Some(lines as usize),
                _ => {
                    return ToolResult::Failure(
                        "max_lines must be a non-negative integer".to_string(),
                    );
                }
            },
            None => None,
        };
        let full_path = self.proj_dir.join(file_path_str);
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

    fn write_tool(&self, args: &Value) -> ToolResult {
        let file_path_str = match args.get(ArgName::FilePath).and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return Self::required_arg(ArgName::FilePath),
        };
        let contents = match args.get(ArgName::Content).and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return Self::required_arg(ArgName::Content),
        };
        let overwrite = match args.get(ArgName::Overwrite) {
            None => false,
            Some(value) => match value.as_bool() {
                Some(value) => value,
                None => return ToolResult::Failure("overwrite must be a boolean".to_string()),
            },
        };
        let full_path = self.proj_dir.join(file_path_str);
        let mut options = std::fs::OpenOptions::new();
        options.write(true);
        if overwrite {
            options.create(true).truncate(true);
        } else {
            options.create_new(true);
        }
        match options.open(full_path) {
            Ok(mut file) => match file.write_all(contents.as_bytes()) {
                Ok(()) => ToolResult::Text("File written successfully".to_string()),
                Err(e) => ToolResult::Error(format!("Error writing file: {}", e)),
            },
            Err(e) if e.kind() == ErrorKind::AlreadyExists => ToolResult::Error(
                "write tool cannot overwrite an existing file unless overwrite is true".to_string(),
            ),
            Err(e) => ToolResult::Error(format!("Error creating file: {}", e)),
        }
    }

    fn edit_tool(&self, args: &Value) -> ToolResult {
        let file_path_str = match args.get(ArgName::FilePath).and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return Self::required_arg(ArgName::FilePath),
        };
        let find = match args.get(ArgName::OldString).and_then(|v| v.as_str()) {
            Some(f) => f,
            None => {
                return Self::required_arg(ArgName::OldString);
            }
        };
        let replace = match args.get(ArgName::NewString).and_then(|v| v.as_str()) {
            Some(r) => r,
            None => {
                return Self::required_arg(ArgName::NewString);
            }
        };
        let replace_all = match args.get(ArgName::ReplaceAll) {
            None => false,
            Some(value) => match value.as_bool() {
                Some(value) => value,
                None => return ToolResult::Failure("replace_all must be a boolean".to_string()),
            },
        };
        let full_path = self.proj_dir.join(file_path_str);
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
        if !replace_all && match_count > 1 {
            return ToolResult::Error(format!(
                "old_string matched {} times. Set replace_all to true to replace every occurrence, or include more surrounding context to make the match unique.",
                match_count
            ));
        }

        let updated = content.replace(find, replace);
        let relative_path = full_path
            .strip_prefix(&self.proj_dir)
            .unwrap_or(&full_path)
            .to_string_lossy()
            .into_owned();
        let diff = DiffView::from_text(relative_path, &content, &updated);
        match std::fs::write(full_path, updated) {
            Ok(()) => ToolResult::FileEdit {
                message: format!("File edited successfully. Made {match_count} replacements"),
                diff,
            },
            Err(e) => ToolResult::Error(format!("Error writing file: {}", e)),
        }
    }

    async fn bash_tool(&self, args: &Value) -> ToolResult {
        let command = match args.get(ArgName::Command).and_then(|v| v.as_str()) {
            Some(c) => {
                if c.is_empty() {
                    return Self::required_arg(ArgName::Command);
                }
                c
            }
            None => return Self::required_arg(ArgName::Command),
        };
        match Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&self.proj_dir)
            .output()
            .await
        {
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

    async fn websearch_tool(args: &Value) -> ToolResult {
        let query = match args.get(ArgName::Query).and_then(Value::as_str) {
            Some(q) => q,
            None => return Self::required_arg(ArgName::Query),
        };
        let max_results = match args.get(ArgName::MaxResults) {
            None => 5,
            Some(v) => match v.as_u64().filter(|&n| n > 0 && n <= u32::MAX as u64) {
                Some(n) => n as u32,
                None => {
                    return ToolResult::Failure(
                        "max_results must be a positive integer".to_string(),
                    );
                }
            },
        };
        let results = web_search(SearchOptions {
            query: query.to_string(),
            max_results: Some(max_results),
            provider: Box::new(DuckDuckGoProvider::new()),
            ..Default::default()
        })
        .await;

        match results {
            Ok(results) => ToolResult::Text(
                results
                    .iter()
                    .map(|r| {
                        format!(
                            "{}\n{}\n{}",
                            r.title,
                            r.url,
                            r.snippet.as_deref().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            Err(e) => ToolResult::Error(format!("Web search failed: {e}")),
        }
    }

    async fn webfetch_tool(args: &Value) -> ToolResult {
        let url_str = match args.get(ArgName::Url).and_then(|v| v.as_str()) {
            Some(u) => u,
            None => return Self::required_arg(ArgName::Url),
        };

        let parsed = match Url::parse(url_str) {
            Ok(u) => u,
            Err(e) => return ToolResult::Failure(format!("Invalid url: {e}")),
        };

        match parsed.scheme() {
            "http" | "https" => {}
            other => return ToolResult::Failure(format!("Unsupported url scheme: {other}")),
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
        let pattern = match args.get(ArgName::Query).and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return Self::required_arg(ArgName::Query),
        };
        let path_prefix = args
            .get(ArgName::Path)
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let root = if path_prefix.starts_with('/') {
            PathBuf::from(path_prefix)
        } else {
            self.proj_dir.join(path_prefix)
        };

        let walker = match Self::create_file_walker(&root, Some(pattern)) {
            Ok(walker) => walker,
            Err(error) => return error,
        };
        let mut hits: Vec<(SystemTime, PathBuf)> = Vec::new();
        let mut timed_out = false;
        let start = Instant::now();
        for entry in walker {
            if start.elapsed() >= Duration::from_secs(10) {
                timed_out = true;
                break;
            }
            let Ok(entry) = entry else { continue };
            if !entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
            {
                continue;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            hits.push((modified, entry.into_path()));
        }

        hits.sort_by_key(|i| Reverse(i.0));
        const LIMIT: usize = 100;
        let truncated = timed_out || hits.len() > LIMIT;
        hits.truncate(LIMIT);
        let mut results: Vec<String> = hits
            .into_iter()
            .map(|(_, path)| {
                path.strip_prefix(&self.proj_dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        if truncated {
            results.push(
                "[Warning: glob results were limited to 100 files or 10 seconds. Please provide a more specific search pattern.]"
                    .to_string(),
            );
        }
        ToolResult::Text(results.join("\n"))
    }

    fn ask_tool(&self, args: &Value) -> ToolResult {
        let title = match args.get(ArgName::Question).and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return Self::required_arg(ArgName::Question),
        };
        let mut options: Vec<String> = args
            .get(ArgName::Options)
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
        if args.get(ArgName::Plan).is_none() {
            return Self::required_arg(ArgName::Plan);
        }

        ToolResult::Text("Plan created".to_string())
    }

    fn grep_tool(&self, args: &Value) -> ToolResult {
        let pattern_str = match args.get(ArgName::Query).and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return Self::required_arg(ArgName::Query),
        };
        let path_str = args
            .get(ArgName::Path)
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let include_pattern = args.get(ArgName::FilePattern).and_then(|v| v.as_str());
        let recursive = args
            .get(ArgName::Recursive)
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let full_path = if path_str.starts_with('/') {
            PathBuf::from(path_str)
        } else {
            self.proj_dir.join(path_str)
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

    fn grep_file(&self, path: &Path, regex: &Regex, results: &mut Vec<String>) {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        for (line_num, line) in content.lines().enumerate() {
            if regex.is_match(line) {
                let display_path = path.strip_prefix(&self.proj_dir).unwrap_or(path);
                results.push(format!(
                    "{}:{}: {}",
                    display_path.to_string_lossy(),
                    line_num + 1,
                    line.trim_start()
                ));
            }
        }
    }

    fn grep_directory(
        &self,
        dir: &Path,
        regex: &Regex,
        include_pattern: Option<&str>,
        results: &mut Vec<String>,
    ) {
        let Ok(walker) = Self::create_file_walker(dir, include_pattern) else {
            return;
        };
        for entry in walker.filter_map(Result::ok) {
            if entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
            {
                self.grep_file(entry.path(), regex, results);
            }
        }
    }

    fn create_file_walker(path: &Path, pattern: Option<&str>) -> Result<Walk, ToolResult> {
        let overrides = if let Some(pattern) = pattern {
            let mut builder = OverrideBuilder::new(path);
            if let Err(e) = builder.add(pattern) {
                return Err(ToolResult::Error(format!("Error in glob pattern: {e}")));
            }
            match builder.build() {
                Ok(overrides) => Some(overrides),
                Err(e) => return Err(ToolResult::Error(format!("Error in glob pattern: {e}"))),
            }
        } else {
            None
        };

        let mut builder = WalkBuilder::new(path);
        if let Some(overrides) = overrides {
            builder.overrides(overrides);
        }
        Ok(builder
            .hidden(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .require_git(false)
            .follow_links(false)
            .build())
    }
}
