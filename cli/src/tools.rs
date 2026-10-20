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
    pub const Name: &'static str = "name";
    pub const NewString: &'static str = "new_string";
    pub const OldString: &'static str = "old_string";
    pub const Options: &'static str = "options";
    pub const Path: &'static str = "path";
    pub const Plan: &'static str = "plan";
    pub const Query: &'static str = "query";
    pub const Question: &'static str = "question";
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
            ToolName::Glob => &[ArgName::Path, ArgName::Query],
            ToolName::Grep => &[ArgName::Path, ArgName::FilePattern, ArgName::Query],
            ToolName::Question => &[ArgName::Question],
            ToolName::Plan => &[],
            ToolName::Skill => &[ArgName::Name],
            ToolName::WebFetch => &[ArgName::Url],
        }
    }

    pub fn genai_tool(self) -> Tool {
        match self {
            ToolName::ReadFile => Tool::new(self.as_str())
                .with_description("Read text from a workspace file; read the whole file or request a line range. Paths are resolved from the current working directory.")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::FilePath: {
                            "type": "string",
                            "description": "Path to the file, relative to the current working directory (or an absolute path)"
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
                .with_description("Replace one exact, unique text match in an existing workspace file. Include enough surrounding context to identify the intended match.")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::FilePath: {
                            "type": "string",
                            "description": "Path to the existing file, relative to the current working directory (or an absolute path)"
                        },
                        ArgName::OldString: {
                            "type": "string",
                            "description": "Exact text to locate in the file, including whitespace and indentation. It must occur exactly once; include surrounding context if needed."
                        },
                        ArgName::NewString: {
                            "type": "string",
                            "description": "Text that will replace the unique old_string match"
                        }
                    },
                    "required": [ArgName::FilePath, ArgName::OldString, ArgName::NewString]
                })),
            ToolName::CreateFile => Tool::new(self.as_str())
                .with_description("Create a new file in the workspace. Fails if the target file already exists; use edit_file to modify an existing file.")
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
                .with_description("Run a shell command with bash -c in the current workspace and return its exit status, stdout, and stderr.")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Command: {
                            "type": "string",
                            "description": "Shell command string to run with bash -c"
                        }
                    },
                    "required": [ArgName::Command]
                })),
            ToolName::Glob => Tool::new(self.as_str())
                .with_description("Find workspace files whose paths match a glob pattern.")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Query: {
                            "type": "string",
                            "description": "Glob pattern to match file paths (for example, '**/*.rs' or 'src/**/*.toml')"
                        },
                        ArgName::Path: {
                            "type": "string",
                            "description": "Optional directory to search in (defaults to current directory)"
                        }
                    },
                    "required": [ArgName::Query]
                })),
            ToolName::Grep => Tool::new(self.as_str())
                .with_description("Search file contents for a regular-expression pattern, optionally restricting the search to a path and matching files.")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Query: {
                            "type": "string",
                            "description": "Regular expression to search for in file contents"
                        },
                        ArgName::Path: {
                            "type": "string",
                            "description": "File or directory path to search"
                        },
                        ArgName::FilePattern: {
                            "type": "string",
                            "description": "Optional glob pattern to filter which files to search (e.g., '*.rs')"
                        },
                        ArgName::Recursive: {
                            "type": "boolean",
                            "description": "Whether to search recursively in directories (default: true)"
                        }
                    },
                    "required": [ArgName::Query]
                })),
            ToolName::Skill => Tool::new(self.as_str())
                .with_description("Load a named skill and return its instructions for use in the current task.")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Name: {
                            "type": "string",
                            "description": "Name of the skill to load"
                        }
                    },
                    "required": [ArgName::Name],
                })),
            ToolName::Question => Tool::new(self.as_str())
                .with_description("Ask the user a question, optionally providing choices to select from.")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Question: {
                            "type": "string",
                            "description": "The question or prompt to show the user"
                        },
                        ArgName::Options: {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional list of choices for the user"
                        }
                    },
                    "required": [ArgName::Question]
                })),
            ToolName::Plan => Tool::new(self.as_str())
                .with_description("Create or update the structured plan for the current task.")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        ArgName::Plan: {
                            "type": "string",
                            "description": "The content of the plan"
                        }
                    },
                    "required": [ArgName::Plan],
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
