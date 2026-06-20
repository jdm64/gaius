/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use genai::chat::{ContentPart, CustomPart};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use similar::{ChangeTag, TextDiff};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffView {
    pub file_path: String,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
    pub missing_newline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffLineKind {
    Context,
    Delete,
    Insert,
}

impl DiffView {
    pub fn from_text(file_path: String, before: &str, after: &str) -> Self {
        let diff = TextDiff::from_lines(before, after);
        let hunks = diff
            .grouped_ops(3)
            .into_iter()
            .map(|group| {
                let mut lines = Vec::new();
                for op in &group {
                    for change in diff.iter_changes(op) {
                        let kind = match change.tag() {
                            ChangeTag::Delete => DiffLineKind::Delete,
                            ChangeTag::Insert => DiffLineKind::Insert,
                            ChangeTag::Equal => DiffLineKind::Context,
                        };
                        let value = change.value();
                        lines.push(DiffLine {
                            kind,
                            old_line: change.old_index().map(|idx| idx + 1),
                            new_line: change.new_index().map(|idx| idx + 1),
                            text: strip_line_ending(value).to_string(),
                            missing_newline: !value.ends_with('\n'),
                        });
                    }
                }

                let old_start = lines.iter().find_map(|line| line.old_line).unwrap_or(0);
                let new_start = lines.iter().find_map(|line| line.new_line).unwrap_or(0);
                let old_lines = lines.iter().filter(|line| line.old_line.is_some()).count();
                let new_lines = lines.iter().filter(|line| line.new_line.is_some()).count();

                DiffHunk {
                    old_start,
                    old_lines,
                    new_start,
                    new_lines,
                    lines,
                }
            })
            .collect();

        Self { file_path, hunks }
    }

    pub fn from_marker(data: &Value) -> Option<DiffView> {
        if data.get("kind").and_then(|value| value.as_str()) != Some("diff_view") {
            return None;
        }
        if data.get("version").and_then(|value| value.as_u64()) != Some(1) {
            return None;
        }
        serde_json::from_value(data.clone()).ok()
    }

    pub fn to_part(&self) -> ContentPart {
        ContentPart::Custom(CustomPart {
            model_iden: None,
            data: json!({
                "kind": "diff_view",
                "version": 1,
                "file_path": self.file_path,
                "hunks": self.hunks,
            }),
        })
    }
}

fn strip_line_ending(value: &str) -> &str {
    value
        .strip_suffix("\r\n")
        .or_else(|| value.strip_suffix('\n'))
        .unwrap_or(value)
}
