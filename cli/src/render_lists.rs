/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::render::Render;
use crate::{
    agents::AgentDefinition,
    commands::Command,
    input::{FileEntry, PickList, ProviderInfoRow},
    models::ModelPickerRow,
    session::Session,
};
use ratatui::Frame;
use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Clear, List, ListItem, Padding},
};

struct PickListRenderSpec {
    title: &'static str,
    max_width: u16,
    empty_text: &'static str,
    background: Style,
}

impl Render {
    pub fn draw_commands(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        picker: &PickList<Command>,
    ) -> Option<Vec<(&'static str, &'static str)>> {
        self.draw_pick_list(
            frame,
            area,
            picker,
            PickListRenderSpec {
                title: "Commands",
                max_width: 50,
                empty_text: "No matching commands",
                background: Style::default(),
            },
            |cmd, _index| ListItem::new(format!("/{} - {}", cmd.name, cmd.description)),
        );
        Some(vec![
            ("Type", "filter"),
            ("Enter", "select"),
            ("Esc", "close"),
        ])
    }

    pub fn draw_sessions(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        picker: &PickList<Session>,
        renaming: bool,
    ) -> Option<Vec<(&'static str, &'static str)>> {
        self.draw_pick_list(
            frame,
            area,
            picker,
            PickListRenderSpec {
                title: "Sessions",
                max_width: 50,
                empty_text: "No sessions",
                background: Style::default(),
            },
            |session, _index| ListItem::new(session.display_name()),
        );
        if renaming {
            Some(vec![("Enter", "save"), ("Esc", "cancel")])
        } else {
            Some(vec![
                ("Enter", "load"),
                ("Ctrl+E", "rename"),
                ("Ctrl+D", "delete"),
                ("Esc", "close"),
            ])
        }
    }

    pub fn draw_models(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        picker: &PickList<ModelPickerRow>,
    ) -> Option<Vec<(&'static str, &'static str)>> {
        let display_rows = Self::model_display_rows(picker);
        let header_color = self.theme.header;
        self.draw_indexed_pick_list(
            frame,
            area,
            picker,
            &display_rows,
            PickListRenderSpec {
                title: "Models",
                max_width: 80,
                empty_text: "No matching models",
                background: Style::default(),
            },
            |row, _index| match row {
                ModelPickerRow::Header(label) => {
                    ListItem::new(label.as_str()).style(Style::default().fg(header_color))
                }
                ModelPickerRow::Separator => ListItem::new(""),
                ModelPickerRow::Model(model) | ModelPickerRow::RecentModel(model) => {
                    ListItem::new(model.label())
                }
            },
        );
        Some(vec![
            ("Type", "filter"),
            ("Enter", "select"),
            ("Ctrl+N", "add provider"),
            ("Ctrl+R", "reload"),
            ("Ctrl+D", "delete"),
            ("Esc", "close"),
        ])
    }

    pub fn draw_add_provider(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        picker: &PickList<ProviderInfoRow>,
        active_input: &str,
    ) -> Option<Vec<(&'static str, &'static str)>> {
        let selected = picker.selected;
        let indices: Vec<usize> = (0..picker.rows.len()).collect();
        self.draw_indexed_pick_list(
            frame,
            area,
            picker,
            &indices,
            PickListRenderSpec {
                title: "Add Provider",
                max_width: 80,
                empty_text: "",
                background: Style::default(),
            },
            |row, index| {
                let display_value = if index == selected {
                    active_input.to_string()
                } else {
                    row.masked_value()
                };
                ListItem::new(format!("{}: {}", row.label(), display_value))
            },
        );
        Some(vec![
            ("Type", "edit"),
            ("Up/Down", "field"),
            ("Enter", "validate/save"),
            ("Esc", "cancel"),
        ])
    }

    pub fn draw_agents(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        picker: &PickList<AgentDefinition>,
    ) -> Option<Vec<(&'static str, &'static str)>> {
        self.draw_pick_list(
            frame,
            area,
            picker,
            PickListRenderSpec {
                title: "Agents",
                max_width: 60,
                empty_text: "No matching agents",
                background: Style::default(),
            },
            |agent, _index| ListItem::new(agent.name.as_str()),
        );
        Some(vec![
            ("Type", "filter"),
            ("Enter", "select"),
            ("Esc", "close"),
        ])
    }

    pub fn draw_files(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        picker: &PickList<FileEntry>,
    ) -> Option<Vec<(&'static str, &'static str)>> {
        self.draw_pick_list(
            frame,
            area,
            picker,
            PickListRenderSpec {
                title: "Files",
                max_width: 60,
                empty_text: "No matching files",
                background: Style::default(),
            },
            |file, _index| ListItem::new(file.name.clone()),
        );
        Some(vec![
            ("Type", "filter"),
            ("Enter", "select"),
            ("Esc", "close"),
        ])
    }

    fn draw_pick_list<'a, T, F>(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        picker: &'a PickList<T>,
        spec: PickListRenderSpec,
        row_item: F,
    ) where
        F: Fn(&'a T, usize) -> ListItem<'a>,
    {
        self.draw_indexed_pick_list(frame, area, picker, &picker.filtered, spec, row_item);
    }

    fn draw_indexed_pick_list<'a, T, F>(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        picker: &'a PickList<T>,
        display_rows: &[usize],
        spec: PickListRenderSpec,
        row_item: F,
    ) where
        F: Fn(&'a T, usize) -> ListItem<'a>,
    {
        let row_count = display_rows.len().max(1);
        let visible_rows = (row_count as u16).clamp(1, 10);
        let width = spec.max_width.min(area.width.saturating_sub(4).max(1));
        let height = visible_rows + 2;
        let x = area.x + 2;
        let y = area.y - height;
        let rect = Rect::new(x, y, width, height);

        let selected_row = picker.selected_row_index().unwrap_or(0);
        let items: Vec<ListItem> = if display_rows.is_empty() {
            vec![ListItem::new(spec.empty_text)]
        } else {
            let selected_display = display_rows
                .iter()
                .position(|row_index| *row_index == selected_row)
                .unwrap_or(0);
            let visible = display_rows.len().clamp(1, 10);
            let start = selected_display.saturating_add(1).saturating_sub(visible);
            let end = (start + visible).min(display_rows.len());
            display_rows[start..end]
                .iter()
                .map(|row_index| {
                    let row_index = *row_index;
                    let mut item = row_item(&picker.rows[row_index], row_index);
                    if row_index == selected_row {
                        item = item.style(Style::default().bg(self.theme.selected));
                    }
                    item
                })
                .collect()
        };

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(spec.title)
                .padding(Padding::horizontal(1))
                .style(spec.background),
        );

        frame.render_widget(Clear, rect);
        frame.render_widget(list, rect);
    }

    fn model_display_rows(picker: &PickList<ModelPickerRow>) -> Vec<usize> {
        if picker.filtered.is_empty() {
            return Vec::new();
        }

        let mut display_rows = Vec::new();
        let selected: std::collections::BTreeSet<usize> = picker.filtered.iter().copied().collect();

        for (index, row) in picker.rows.iter().enumerate() {
            match row {
                ModelPickerRow::Model(_) | ModelPickerRow::RecentModel(_)
                    if selected.contains(&index) =>
                {
                    display_rows.push(index)
                }
                ModelPickerRow::Header(_)
                    if Self::section_has_selected_model(index, picker, &selected) =>
                {
                    display_rows.push(index);
                }
                ModelPickerRow::Separator
                    if Self::separator_has_selected_models(index, picker, &selected) =>
                {
                    display_rows.push(index);
                }
                ModelPickerRow::Header(_)
                | ModelPickerRow::Separator
                | ModelPickerRow::Model(_)
                | ModelPickerRow::RecentModel(_) => {}
            }
        }

        display_rows
    }

    fn section_has_selected_model(
        header_index: usize,
        picker: &PickList<ModelPickerRow>,
        selected: &std::collections::BTreeSet<usize>,
    ) -> bool {
        picker.rows[header_index + 1..]
            .iter()
            .enumerate()
            .take_while(|(_, row)| {
                !matches!(row, ModelPickerRow::Header(_) | ModelPickerRow::Separator)
            })
            .any(|(offset, row)| {
                matches!(
                    row,
                    ModelPickerRow::Model(_) | ModelPickerRow::RecentModel(_)
                ) && selected.contains(&(header_index + 1 + offset))
            })
    }

    fn separator_has_selected_models(
        separator_index: usize,
        picker: &PickList<ModelPickerRow>,
        selected: &std::collections::BTreeSet<usize>,
    ) -> bool {
        let has_before = picker.rows[..separator_index]
            .iter()
            .enumerate()
            .rev()
            .take_while(|(_, row)| !matches!(row, ModelPickerRow::Separator))
            .any(|(index, row)| {
                matches!(
                    row,
                    ModelPickerRow::Model(_) | ModelPickerRow::RecentModel(_)
                ) && selected.contains(&index)
            });
        let has_after =
            picker.rows[separator_index + 1..]
                .iter()
                .enumerate()
                .any(|(offset, row)| {
                    matches!(
                        row,
                        ModelPickerRow::Model(_) | ModelPickerRow::RecentModel(_)
                    ) && selected.contains(&(separator_index + 1 + offset))
                });

        has_before && has_after
    }
}
