/* Copyright 2026 Justin Madru (justin.jdm64@gmail.com)
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use genai::chat::*;
use serde_json::Value;
use serde_json::json;
use std::io::Write;
use std::process::Command;

pub struct ToolEngine {}

impl ToolEngine {
    pub fn build_tools(&self) -> Vec<Tool> {
        vec![
            Tool::new("read_file")
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
            Tool::new("write_file")
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
            Tool::new("edit_file")
                .with_description("Replace one exact string match in a file")
                .with_schema(json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "The path to the file to edit"
                        },
                        "find": {
                            "type": "string",
                            "description": "The exact string to find"
                        },
                        "replace": {
                            "type": "string",
                            "description": "The string to replace the match with"
                        }
                    },
                    "required": ["file_path", "find", "replace"]
                })),
            Tool::new("bash")
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
        ]
    }

    pub fn execute(&self, name: &str, args: &Value) -> String {
        match name {
            "read_file" => self.read_file_tool(args),
            "write_file" => self.write_file_tool(args),
            "edit_file" => self.edit_file_tool(args),
            "bash" => self.bash_tool(args),
            _ => format!("Unknown tool call: {} ({})", name, args),
        }
    }

    fn read_file_tool(&self, args: &Value) -> String {
        let file_path_str = match args.get("file_path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return "Error: Missing file_path".to_string(),
        };
        let start_line = match args.get("start_line") {
            Some(value) => match value.as_u64() {
                Some(line) if line >= 1 => line as usize,
                _ => {
                    return "Error: start_line must be an integer greater than or equal to 1"
                        .to_string();
                }
            },
            None => 1,
        };
        let max_lines = match args.get("max_lines") {
            Some(value) => match value.as_u64() {
                Some(lines) => Some(lines as usize),
                _ => return "Error: max_lines must be a non-negative integer".to_string(),
            },
            None => None,
        };
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => return format!("Error getting current directory: {}", e),
        };
        let full_path = cwd.join(file_path_str);
        match std::fs::read_to_string(full_path) {
            Ok(content) => {
                if start_line == 1 && max_lines.is_none() {
                    return content;
                }

                let lines = content.split_inclusive('\n').skip(start_line - 1);
                match max_lines {
                    Some(max_lines) => lines.take(max_lines).collect(),
                    None => lines.collect(),
                }
            }
            Err(e) => format!("Error reading file: {}", e),
        }
    }

    fn write_file_tool(&self, args: &Value) -> String {
        let file_path_str = match args.get("file_path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return "Error: Missing file_path".to_string(),
        };
        let contents = match args.get("contents").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return "Error: Missing contents".to_string(),
        };
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => return format!("Error getting current directory: {}", e),
        };
        let full_path = cwd.join(file_path_str);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(full_path)
        {
            Ok(mut file) => match file.write_all(contents.as_bytes()) {
                Ok(()) => "File written successfully".to_string(),
                Err(e) => format!("Error writing file: {}", e),
            },
            Err(e) => format!("Error creating file: {}", e),
        }
    }

    fn edit_file_tool(&self, args: &Value) -> String {
        let file_path_str = match args.get("file_path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return "Error: Missing file_path".to_string(),
        };
        let find = match args.get("find").and_then(|v| v.as_str()) {
            Some(f) => f,
            None => return "Error: Missing find".to_string(),
        };
        let replace = match args.get("replace").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return "Error: Missing replace".to_string(),
        };
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => return format!("Error getting current directory: {}", e),
        };
        let full_path = cwd.join(file_path_str);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(content) => content,
            Err(e) => return format!("Error reading file: {}", e),
        };
        let match_count = content.matches(find).count();
        if match_count == 0 {
            return "Error: find string not found".to_string();
        }
        if match_count > 1 {
            return format!("Error: find string matched {} times", match_count);
        }

        let updated = content.replace(find, replace);
        match std::fs::write(full_path, updated) {
            Ok(()) => "File edited successfully".to_string(),
            Err(e) => format!("Error writing file: {}", e),
        }
    }

    fn bash_tool(&self, args: &Value) -> String {
        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return "Error: Missing command".to_string(),
        };
        match Command::new("bash").arg("-c").arg(command).output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                format!(
                    "Exit status: {}\nstdout:\n{}\nstderr:\n{}",
                    output.status, stdout, stderr
                )
            }
            Err(e) => format!("Error executing command: {}", e),
        }
    }
}
