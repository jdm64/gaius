/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use genai::chat::Tool;
use serde_json::json;

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

    pub fn genai_tool(self) -> Tool {
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
                            "description": "one-based line number to start reading from; defaults to 1"
                        },
                        "max_lines": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "maximum number of lines to read; 0 or omitted reads the entire file"
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
