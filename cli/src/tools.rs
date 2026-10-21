/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use genai::chat::Tool;
use serde_json::json;

#[allow(non_upper_case_globals)]
pub struct ArgName;

#[allow(non_upper_case_globals)]
impl ArgName {
    pub const Command: &'static str = "command";
    pub const Content: &'static str = "content";
    pub const FilePath: &'static str = "file_path";
    pub const FilePattern: &'static str = "file_pattern";
    pub const MaxLines: &'static str = "max_lines";
    pub const MaxResults: &'static str = "max_results";
    pub const Name: &'static str = "name";
    pub const NewString: &'static str = "new_string";
    pub const OldString: &'static str = "old_string";
    pub const Options: &'static str = "options";
    pub const Overwrite: &'static str = "overwrite";
    pub const Path: &'static str = "path";
    pub const Plan: &'static str = "plan";
    pub const Query: &'static str = "query";
    pub const Question: &'static str = "question";
    pub const Recursive: &'static str = "recursive";
    pub const StartLine: &'static str = "start_line";
    pub const Url: &'static str = "url";
}

const FILE_PATH_DESC: &str = "Workspace relative or absolute file path";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolName {
    Read,
    Write,
    Edit,
    Bash,
    Glob,
    Grep,
    Question,
    Plan,
    Skill,
    WebFetch,
    WebSearch,
}

impl ToolName {
    pub const ALL: [ToolName; 11] = [
        ToolName::Read,
        ToolName::Write,
        ToolName::Edit,
        ToolName::Bash,
        ToolName::Glob,
        ToolName::Grep,
        ToolName::Question,
        ToolName::Plan,
        ToolName::Skill,
        ToolName::WebFetch,
        ToolName::WebSearch,
    ];

    pub fn name(self) -> String {
        self.as_str().to_string()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ToolName::Read => "read",
            ToolName::Write => "write",
            ToolName::Edit => "edit",
            ToolName::Bash => "bash",
            ToolName::Glob => "glob",
            ToolName::Grep => "grep",
            ToolName::Question => "question",
            ToolName::Plan => "plan",
            ToolName::Skill => "skill",
            ToolName::WebFetch => "webfetch",
            ToolName::WebSearch => "websearch",
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
            ToolName::Read => &[ArgName::FilePath, ArgName::StartLine, ArgName::MaxLines],
            ToolName::Write => &[ArgName::FilePath, ArgName::Overwrite],
            ToolName::Edit => &[ArgName::FilePath],
            ToolName::Bash => &[ArgName::Command],
            ToolName::Glob => &[ArgName::Path, ArgName::Query],
            ToolName::Grep => &[ArgName::Path, ArgName::FilePattern, ArgName::Query],
            ToolName::Question => &[ArgName::Question],
            ToolName::Plan => &[],
            ToolName::Skill => &[ArgName::Name],
            ToolName::WebFetch => &[ArgName::Url],
            ToolName::WebSearch => &[ArgName::Query],
        }
    }

    pub fn genai_tool(self) -> Tool {
        match self {
            ToolName::Read => Tool::new(self.as_str())
                .with_description("Read workspace file")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::FilePath: {
                            "type": "string",
                            "minLength": 1,
                            "description": FILE_PATH_DESC
                        },
                        ArgName::StartLine: {
                            "type": "integer",
                            "minimum": 1,
                            "description": "First 1-based line to read (default: 1)"
                        },
                        ArgName::MaxLines: {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Line limit; 0 or omitted reads the whole file"
                        }
                    },
                    "required": [ArgName::FilePath]
                })),
            ToolName::Edit => Tool::new(self.as_str())
                .with_description("Replace one unique exact text match in a file")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::FilePath: {
                            "type": "string",
                            "minLength": 1,
                            "description": FILE_PATH_DESC
                        },
                        ArgName::OldString: {
                            "type": "string",
                            "description": "Exact text to match including whitespace; must occur once"
                        },
                        ArgName::NewString: {
                            "type": "string",
                            "description": "Replacement text"
                        }
                    },
                    "required": [ArgName::FilePath, ArgName::OldString, ArgName::NewString]
                })),
            ToolName::Write => Tool::new(self.as_str())
                .with_description("Create or replace file")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::FilePath: {
                            "type": "string",
                            "minLength": 1,
                            "description": FILE_PATH_DESC
                        },
                        ArgName::Content: {
                            "type": "string",
                            "description": "Contents to write to the file"
                        },
                        ArgName::Overwrite: {
                            "type": "boolean",
                            "description": "Overwrite if it exists (default: false)"
                        }
                    },
                    "required": [ArgName::FilePath, ArgName::Content]
                })),
            ToolName::Bash => Tool::new(self.as_str())
                .with_description("Execute shell command in the workspace")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Command: {
                            "type": "string",
                            "minLength": 1,
                            "description": "Shell command string to run"
                        }
                    },
                    "required": [ArgName::Command]
                })),
            ToolName::Glob => Tool::new(self.as_str())
                .with_description("Find files matching glob")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Query: {
                            "type": "string",
                            "minLength": 1,
                            "description": "Glob pattern (e.g. '**/*.rs')"
                        },
                        ArgName::Path: {
                            "type": "string",
                            "description": "Directory to search in (default: current directory)"
                        }
                    },
                    "required": [ArgName::Query]
                })),
            ToolName::Grep => Tool::new(self.as_str())
                .with_description("Search file contents with regular expression")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Query: {
                            "type": "string",
                            "minLength": 1,
                            "description": "Regex to search for"
                        },
                        ArgName::Path: {
                            "type": "string",
                            "description": "File or directory to search (default: current directory)"
                        },
                        ArgName::FilePattern: {
                            "type": "string",
                            "description": "Glob filter (e.g. '*.rs')"
                        },
                        ArgName::Recursive: {
                            "type": "boolean",
                            "description": "Search subdirectories (default: true)"
                        }
                    },
                    "required": [ArgName::Query]
                })),
            ToolName::Skill => Tool::new(self.as_str())
                .with_description("Load skill instructions")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Name: {
                            "type": "string",
                            "minLength": 1,
                            "description": "Name of skill to load"
                        }
                    },
                    "required": [ArgName::Name],
                })),
            ToolName::Question => Tool::new(self.as_str())
                .with_description("Ask user a question, optionally with choices")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Question: {
                            "type": "string",
                            "minLength": 1,
                            "description": "Question to ask"
                        },
                        ArgName::Options: {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of choices"
                        }
                    },
                    "required": [ArgName::Question]
                })),
            ToolName::Plan => Tool::new(self.as_str())
                .with_description("Create or update task plan")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Plan: {
                            "type": "string",
                            "minLength": 1,
                            "description": "Plan text"
                        }
                    },
                    "required": [ArgName::Plan],
                })),
            ToolName::WebFetch => Tool::new(self.as_str())
                .with_description("Fetch URL as Markdown")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Url: {
                            "type": "string",
                            "minLength": 1,
                            "description": "HTTP or HTTPS URL"
                        },
                    },
                    "required": [ArgName::Url]
                })),
            ToolName::WebSearch => Tool::new(self.as_str())
                .with_description("Search the web and return ranked results")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Query: {
                            "type": "string",
                            "minLength": 1,
                            "description": "Search terms or question"
                        },
                        ArgName::MaxResults: {
                            "type": "integer",
                            "description": "Result limit (default: 5)",
                            "minimum": 1
                        }
                    },
                    "required": [ArgName::Query]
                })),
        }
    }
}
