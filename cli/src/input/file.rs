/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::input::{InputMode, PromptEditor, picklist::PickList};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
}

impl PromptEditor {
    pub async fn handle_files(
        &mut self,
        key: KeyEvent,
        mut picker: PickList<FileEntry>,
    ) -> InputMode {
        self.handle_input_cursor(key);
        match key.code {
            KeyCode::Esc => return InputMode::PromptInput,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return InputMode::Exit;
            }
            KeyCode::Up => {
                picker.move_up();
            }
            KeyCode::Down => {
                picker.move_down();
            }
            KeyCode::Enter => {
                if let Some(file) = picker.selected_row() {
                    let cursor_byte = char_pos_to_byte_index(&self.input, self.cursor);
                    let token_start = self.input[..cursor_byte]
                        .char_indices()
                        .rev()
                        .find_map(|(index, ch)| ch.is_whitespace().then_some(index + ch.len_utf8()))
                        .unwrap_or(0);
                    self.input = replace_file_query(&file.name, &self.input, self.cursor);
                    self.cursor =
                        self.input[..token_start].chars().count() + file.name.chars().count();
                }
                return InputMode::PromptInput;
            }
            _ if Self::input_changed_key(key) => {
                if let Some(query) = Self::get_file_query(&self.input, self.cursor) {
                    let filtered = Self::filter_files(&query, &picker.rows);
                    picker.replace_filter(filtered);
                } else {
                    return InputMode::PromptInput;
                }
            }
            _ => {}
        }

        InputMode::Files { picker }
    }

    pub fn filter_files(input: &str, files: &[FileEntry]) -> Vec<usize> {
        let query = input
            .strip_prefix('@')
            .unwrap_or(input)
            .trim()
            .to_lowercase();
        files
            .iter()
            .enumerate()
            .filter_map(|(index, file)| {
                (query.is_empty() || file.name.to_lowercase().contains(&query)).then_some(index)
            })
            .collect()
    }

    pub fn get_file_query(input: &str, cursor_pos: usize) -> Option<String> {
        let cursor_byte = char_pos_to_byte_index(input, cursor_pos);
        let input_before_cursor = &input[..cursor_byte];
        let query_start = input_before_cursor
            .char_indices()
            .rev()
            .find_map(|(index, ch)| ch.is_whitespace().then_some(index + ch.len_utf8()))
            .unwrap_or(0);

        input_before_cursor[query_start..]
            .strip_prefix('@')
            .map(ToString::to_string)
    }

    pub fn list_files() -> Vec<FileEntry> {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut entries: Vec<FileEntry> = Vec::new();

        walk_dir(&current_dir, "", &mut entries);

        entries.sort_by(|l, r| l.name.cmp(&r.name));
        entries
    }
}

fn char_pos_to_byte_index(input: &str, char_pos: usize) -> usize {
    input
        .char_indices()
        .nth(char_pos)
        .map(|(index, _)| index)
        .unwrap_or(input.len())
}

pub fn replace_file_query(filename: &str, input: &str, cursor_pos: usize) -> String {
    let cursor_byte = char_pos_to_byte_index(input, cursor_pos);
    let token_start = input[..cursor_byte]
        .char_indices()
        .rev()
        .find_map(|(index, ch)| ch.is_whitespace().then_some(index + ch.len_utf8()))
        .unwrap_or(0);

    if !input[token_start..].starts_with('@') {
        return input.to_string();
    }

    let token_end = input[cursor_byte..]
        .char_indices()
        .find_map(|(index, ch)| ch.is_whitespace().then_some(cursor_byte + index))
        .unwrap_or(input.len());

    format!(
        "{}{}{}",
        &input[..token_start],
        filename,
        &input[token_end..]
    )
}

fn walk_dir(path: &PathBuf, relative_path: &str, entries: &mut Vec<FileEntry>) {
    if let Ok(read_dir) = std::fs::read_dir(path) {
        for entry in read_dir.filter_map(|e| e.ok()) {
            let entry_path = entry.path();
            let name = entry_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            // Skip ignored directories
            if [".git", "target", "node_modules", "build", "dist", "bin"]
                .iter()
                .any(|&ignored| name == ignored)
            {
                continue;
            }

            let rel_name = if relative_path.is_empty() {
                name
            } else {
                format!("{}/{}", relative_path, name)
            };
            entries.push(FileEntry {
                name: rel_name.clone(),
                path: entry_path.clone(),
            });
            if entry_path.is_dir() {
                walk_dir(&entry_path, &rel_name, entries);
            }
        }
    }
}
