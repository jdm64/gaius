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
    pub const Include: &'static str = "include";
    pub const MaxLines: &'static str = "max_lines";
    pub const Name: &'static str = "name";
    pub const NewString: &'static str = "new_string";
    pub const OldString: &'static str = "old_string";
    pub const Options: &'static str = "options";
    pub const Path: &'static str = "path";
    pub const Pattern: &'static str = "pattern";
    pub const Prompt: &'static str = "prompt";
    pub const Recursive: &'static str = "recursive";
    pub const StartLine: &'static str = "start_line";
    pub const Url: &'static str = "url";
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

    pub fn name(self) -> String {
        self.as_str().to_string()
    }

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
            ToolName::ReadFile => &[ArgName::FilePath, ArgName::StartLine, ArgName::MaxLines],
            ToolName::CreateFile => &[ArgName::FilePath],
            ToolName::EditFile => &[ArgName::FilePath],
            ToolName::Bash => &[ArgName::Command],
            ToolName::Glob => &[ArgName::Path, ArgName::Pattern],
            ToolName::Grep => &[ArgName::Path, ArgName::Include, ArgName::Pattern],
            ToolName::Question => &[ArgName::Prompt],
            ToolName::Plan => &[],
            ToolName::Skill => &[ArgName::Name],
            ToolName::WebFetch => &[ArgName::Url],
        }
    }

    pub fn genai_tool(self) -> Tool {
        match self {
            ToolName::ReadFile => Tool::new(self.as_str())
                .with_description("Read the contents of a file")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::FilePath: {
                            "type": "string",
                            "description": "The path to the file to read"
                        },
                        ArgName::StartLine: {
                            "type": "integer",
                            "minimum": 1,
                            "description": "one-based line number to start reading from; defaults to 1"
                        },
                        ArgName::MaxLines: {
                            "type": "integer",
                            "minimum": 0,
                            "description": "maximum number of lines to read; 0 or omitted reads the entire file"
                        }
                    },
                    "required": [ArgName::FilePath]
                })),
            ToolName::EditFile => Tool::new(self.as_str())
                .with_description("Modify an existing file by replacing exactly one string match")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::FilePath: {
                            "type": "string",
                            "description": "The path to the file to edit"
                        },
                        ArgName::OldString: {
                            "type": "string",
                            "description": "The exact string to find and replace"
                        },
                        ArgName::NewString: {
                            "type": "string",
                            "description": "The string to replace the match with"
                        }
                    },
                    "required": [ArgName::FilePath, ArgName::OldString, ArgName::NewString]
                })),
            ToolName::CreateFile => Tool::new(self.as_str())
                .with_description("Create a new file with the provided contents")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::FilePath: {
                            "type": "string",
                            "description": "The path to the file to create"
                        },
                        ArgName::Content: {
                            "type": "string",
                            "description": "The contents to write to the new file"
                        }
                    },
                    "required": [ArgName::FilePath, ArgName::Content]
                })),
            ToolName::Bash => Tool::new(self.as_str())
                .with_description("Execute a bash command")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Command: {
                            "type": "string",
                            "description": "The bash command to execute"
                        }
                    },
                    "required": [ArgName::Command]
                })),
            ToolName::Glob => Tool::new(self.as_str())
                .with_description("Find files matching a glob pattern")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Pattern: {
                            "type": "string",
                            "description": "Glob pattern to match files (e.g., '**/*.rs', 'src/**/*.toml')"
                        },
                        ArgName::Path: {
                            "type": "string",
                            "description": "Optional directory to search in (defaults to current directory)"
                        }
                    },
                    "required": [ArgName::Pattern]
                })),
            ToolName::Grep => Tool::new(self.as_str())
                .with_description("Search file contents using regex pattern")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Pattern: {
                            "type": "string",
                            "description": "Regex pattern to search for"
                        },
                        ArgName::Path: {
                            "type": "string",
                            "description": "File or directory path to search"
                        },
                        ArgName::Include: {
                            "type": "string",
                            "description": "Optional glob pattern to filter which files to search (e.g., '*.rs')"
                        },
                        ArgName::Recursive: {
                            "type": "boolean",
                            "description": "Whether to search recursively in directories (default: true)"
                        }
                    },
                    "required": [ArgName::Pattern]
                })),
            ToolName::Skill => Tool::new(self.as_str())
                .with_description("Invoke a skill by name to get its instructions")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Name: {
                            "type": "string",
                            "description": "Name of skill to invoke"
                        }
                    },
                    "required": [ArgName::Name],
                })),
            ToolName::Question => Tool::new(self.as_str())
                .with_description("Ask the user a question with optional choices")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Prompt: {
                            "type": "string",
                            "description": "The question or prompt to show the user"
                        },
                        ArgName::Options: {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional list of choices for the user"
                        }
                    },
                    "required": [ArgName::Prompt]
                })),
            ToolName::Plan => Tool::new(self.as_str())
                .with_description("Create a structured markdown formatted plan")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Content: {
                            "type": "string",
                            "description": "The content of the plan"
                        }
                    },
                    "required": [ArgName::Content],
                })),
            ToolName::WebFetch => Tool::new(self.as_str())
                .with_description("Fetch a URL and return its content as cleaned Markdown text")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Url: {
                            "type": "string",
                            "description": "The fully-qualified http(s) URL to fetch"
                        },
                    },
                    "required": [ArgName::Url]
                })),
        }
    }
}
