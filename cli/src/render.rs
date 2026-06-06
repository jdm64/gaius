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

use crate::{
    agents::AgentDefinition,
    commands::Command,
    input::{FileEntry, InputMode, PickList, ProviderInfoRow},
    models::ModelPickerRow,
    session::Session,
    tui::{TuiApp, TuiMessage, wrapped_line_count},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap},
};
use serde_json::{self, Value, from_str};
use tui_markdown::{Options, from_str_with_options};

pub const INPUT_HEIGHT: u16 = 3;
const INPUT_PROMPT_PREFIX: &str = "> ";
const USER_PROMPT_BAR: &str = "\u{2503} ";

struct PickListRenderSpec {
    title: &'static str,
    max_width: u16,
    empty_text: &'static str,
    background: Style,
}

enum QuestionRow {
    Title(String),
    Blank,
    Option(usize, String),
}

struct ColorTheme {
    header: Color,
    selected: Color,
    thinking: Color,
    user_bar: Color,
    user_box: Color,
    inputbox: Color,
    toolcall: Color,
    error: Color,
}

impl Default for ColorTheme {
    fn default() -> Self {
        ColorTheme {
            header: Color::Yellow,
            selected: Color::Magenta,
            thinking: Color::LightBlue,
            user_bar: Color::Magenta,
            user_box: Color::Rgb(64, 64, 64),
            inputbox: Color::Rgb(64, 0, 64),
            toolcall: Color::Cyan,
            error: Color::Red,
        }
    }
}

pub struct Render {
    theme: ColorTheme,
}

impl Default for Render {
    fn default() -> Self {
        Self::new()
    }
}

impl Render {
    pub fn new() -> Self {
        Render {
            theme: ColorTheme::default(),
        }
    }

    pub fn draw(&self, app: &mut TuiApp, frame: &mut Frame<'_>) {
        let area = frame.area();
        let input_width = area.width - 4;
        let input_prompt = self.input_prompt_lines(app.input.clone(), input_width);
        let input_height = 2 + input_prompt.len() as u16;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(input_height),
                Constraint::Length(1),
            ])
            .split(area);

        frame.render_widget(Clear, area);
        self.draw_history(app, frame, chunks[0]);
        self.draw_input(app, frame, chunks[1], input_prompt, input_width as usize);

        let active_help: Option<Vec<(&'static str, &'static str)>> = match &app.mode {
            InputMode::Command { picker } => self.draw_commands(frame, chunks[1], picker),
            InputMode::Session { picker } => self.draw_sessions(frame, chunks[1], picker, false),
            InputMode::SessionRename { picker } => {
                self.draw_sessions(frame, chunks[1], picker, true)
            }
            InputMode::Models { picker } => self.draw_models(frame, chunks[1], picker),
            InputMode::AddProvider { picker } => {
                self.draw_add_provider(frame, chunks[1], picker, app.input.as_str())
            }
            InputMode::Agents { picker } => self.draw_agents(frame, chunks[1], picker),
            InputMode::Files { picker } => self.draw_files(frame, chunks[1], picker),
            InputMode::PromptInput | InputMode::Exit => None,
            InputMode::Question {
                title,
                options,
                selected,
            } => self.draw_question(frame, chunks[1], title, options, *selected),
        };

        let help_items = active_help.unwrap_or(vec![("Ctrl+C", "quit"), ("Ctrl+D", "cancel")]);
        let help_para = Paragraph::new(self.help_spec_to_text(help_items.clone()))
            .block(Block::default().borders(Borders::NONE));
        frame.render_widget(help_para, chunks[2]);
    }

    fn draw_commands(
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

    fn draw_sessions(
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

    fn draw_models(
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

    fn draw_add_provider(
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

    fn draw_agents(
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

    fn draw_files(
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

    fn draw_question(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        title: &str,
        options: &[String],
        selected: usize,
    ) -> Option<Vec<(&'static str, &'static str)>> {
        let wrap_width: usize = 70usize.saturating_sub(4).max(1);

        let mut rows: Vec<QuestionRow> = Vec::new();
        let mut line_buf = String::new();
        for word in title.split_whitespace() {
            if line_buf.is_empty() {
                line_buf.push_str(word);
            } else if line_buf.len() + 1 + word.len() <= wrap_width {
                line_buf.push(' ');
                line_buf.push_str(word);
            } else {
                rows.push(QuestionRow::Title(std::mem::take(&mut line_buf)));
                line_buf = word.to_string();
            }
        }
        if !line_buf.is_empty() {
            rows.push(QuestionRow::Title(line_buf));
        }
        rows.push(QuestionRow::Blank);
        for (i, opt) in options.iter().enumerate() {
            rows.push(QuestionRow::Option(i, opt.clone()));
        }

        let title_len = rows.len() - options.len();

        let indices: Vec<usize> = (0..rows.len()).collect();
        let picker = PickList {
            selected: title_len + selected,
            rows,
            filtered: indices,
        };

        self.draw_indexed_pick_list(
            frame,
            area,
            &picker,
            &picker.filtered,
            PickListRenderSpec {
                title: "Question",
                max_width: 70,
                empty_text: "",
                background: Style::default().bg(self.theme.inputbox),
            },
            |row, _index| match row {
                QuestionRow::Title(text) => ListItem::new(text.as_str()),
                QuestionRow::Blank => ListItem::new(""),
                QuestionRow::Option(i, text) => ListItem::new(format!("{}) {}", i + 1, text)),
            },
        );
        Some(vec![
            ("Type", "add response"),
            ("Enter", "send"),
            ("Up/Down", "select"),
            ("Tab", "cancel"),
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

    fn draw_history(&self, app: &mut TuiApp, frame: &mut Frame<'_>, area: Rect) {
        let text_width = area.width.saturating_sub(4).max(1);
        let text_height = area.height.saturating_sub(2).max(1);
        app.history_page_size = text_height;

        self.sync_history_lines(app);

        let wrapped_height = wrapped_line_count(&app.history_lines, text_width);
        let max_scroll = wrapped_height.saturating_sub(text_height);
        let clamped_scroll = app.history_scroll.min(max_scroll);
        let start = max_scroll.saturating_sub(clamped_scroll);
        let lines = self.visible_history_lines(
            &app.history_lines,
            text_width,
            start as usize,
            text_height as usize,
        );

        let title = if let Some(tokens) = app.context_tokens {
            format!(" Gaius - {} - {} - {} ", app.model, app.agent_name, tokens)
        } else {
            format!(" Gaius - {} - {} ", app.model, app.agent_name)
        };

        let history = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::TOP)
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(history, area);

        app.history_scroll = clamped_scroll;
    }

    pub fn render_message(
        &self,
        msg: &TuiMessage,
        show_thinking: bool,
        show_token_info: bool,
    ) -> Vec<Line<'static>> {
        match msg {
            TuiMessage::Thinking(text) => {
                if !show_thinking {
                    return vec![
                        Line::from(format!("Thinking... {}", text.len()))
                            .style(Style::default().fg(self.theme.thinking)),
                    ];
                }
                let style = Style::default()
                    .fg(self.theme.thinking)
                    .add_modifier(Modifier::ITALIC)
                    .add_modifier(Modifier::DIM);
                let options = Options::default();
                let lines: Vec<Line> = from_str_with_options(text, &options)
                    .lines
                    .into_iter()
                    .map(|mut line| {
                        line.spans = line
                            .spans
                            .into_iter()
                            .map(|span| Span::styled(span.content, style.patch(span.style)))
                            .collect();
                        Self::owned_line(line)
                    })
                    .collect();
                lines
            }
            TuiMessage::AgentMessage(text) => {
                let options = Options::default();
                from_str_with_options(text, &options)
                    .lines
                    .into_iter()
                    .map(Self::owned_line)
                    .collect()
            }
            TuiMessage::UserPrompt(text) => {
                let style = self.user_prompt_style();
                vec![
                    self.user_prompt_bar_line(),
                    Line::from(vec![
                        Span::styled(USER_PROMPT_BAR, style.fg(self.theme.user_bar)),
                        Span::raw(text.clone()).style(style.italic().bold()),
                    ]),
                    self.user_prompt_bar_line(),
                ]
            }
            TuiMessage::ToolCall {
                name,
                arguments,
                result,
                error,
            } => {
                let style = Style::default().fg(self.theme.toolcall);
                let json_args = from_str::<Value>(arguments).unwrap_or_default();
                let display = match name.as_str() {
                    "read_file" => Self::arguments_json_fields(
                        &json_args,
                        &["file_path", "start_line", "max_lines"],
                    ),
                    "write_file" => Self::arguments_json_fields(&json_args, &["file_path"]),
                    "edit_file" => Self::arguments_json_fields(&json_args, &["file_path"]),
                    "bash" => Self::arguments_json_fields(&json_args, &["command"]),
                    "glob" => Self::arguments_json_fields(&json_args, &["path", "pattern"]),
                    "grep" => {
                        Self::arguments_json_fields(&json_args, &["path", "include", "pattern"])
                    }
                    "question" => Self::arguments_json_fields(&json_args, &["title"]),
                    "plan" | _ => "".to_string(),
                };

                let spans = vec![
                    Span::styled(name.clone(), style.add_modifier(Modifier::BOLD)),
                    Span::raw(" "),
                    Span::styled(display, style.add_modifier(Modifier::ITALIC)),
                ];
                let mut ret = vec![Line::from(spans)];
                if *error {
                    let e_style = Style::default().fg(self.theme.error);
                    let error_lines: Vec<&str> = result.split('\n').collect();
                    for i in error_lines {
                        if !i.is_empty() {
                            ret.push(Line::from(vec![
                                Span::styled(" \u{21B3} ", e_style),
                                Span::styled(i.to_string(), e_style),
                            ]));
                        }
                    }
                }
                Self::render_tool_results(name, &json_args, result, &mut ret, style);
                ret
            }
            TuiMessage::SystemMessage(text) => {
                let style = Style::default()
                    .fg(self.theme.error)
                    .add_modifier(Modifier::BOLD);
                vec![Line::from(text.clone()).style(style)]
            }
            TuiMessage::TokenInfo(text) => {
                if !show_token_info {
                    return vec![];
                }
                let style = Style::default().fg(self.theme.header);
                vec![Line::from(text.clone()).style(style).right_aligned()]
            }
        }
    }

    fn render_tool_results(
        name: &str,
        args: &Value,
        result: &str,
        lines: &mut Vec<Line>,
        style: Style,
    ) {
        match name {
            "question" => {
                let answers = result
                    .split("\n")
                    .map(|l| " - ".to_string() + l)
                    .collect::<Vec<_>>();
                for l in answers {
                    lines.push(Line::from(Span::styled::<String, Style>(l, style)));
                }
            }
            "plan" => {
                let mut md = String::new();
                if let Some(goal) = args.get("goal").and_then(|g| g.as_str()) {
                    md.push_str("# Goal\n\n");
                    md.push_str(goal);
                    md.push_str("\n\n");
                }
                if let Some(context) = args.get("context").and_then(|c| c.as_str()) {
                    md.push_str("# Context\n\n");
                    md.push_str(context);
                    md.push_str("\n\n");
                }
                if let Some(steps) = args.get("steps").and_then(|s| s.as_array()) {
                    for (index, step) in steps.iter().enumerate() {
                        if let Some(step_text) = step.as_str() {
                            md.push_str(&format!("# Step {}\n\n{}\n\n", index + 1, step_text));
                        }
                    }
                }

                let options = Options::default();
                let rendered_lines: Vec<Line> = from_str_with_options(&md, &options)
                    .lines
                    .into_iter()
                    .map(Self::owned_line)
                    .collect();

                lines.push(Line::raw(" "));
                lines.extend(rendered_lines);
                lines.push(Line::raw(" "));
            }
            _ => {}
        }
    }

    fn draw_input(
        &self,
        app: &TuiApp,
        frame: &mut Frame<'_>,
        area: Rect,
        lines: Vec<Line<'static>>,
        width: usize,
    ) {
        let input = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .title(format!(" {} ", app.status))
                    .borders(Borders::ALL)
                    .style(Style::default().bg(self.theme.inputbox))
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(input, area);

        let cursor = app.input_cursor + 2;
        let (row, col) = (cursor / width, cursor % width);
        let cursor_x = area.x.saturating_add(2).saturating_add(col as u16);
        let cursor_y = area.y.saturating_add(1).saturating_add(row as u16);

        frame.set_cursor_position((cursor_x, cursor_y));
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

    fn sync_history_lines(&self, app: &mut TuiApp) {
        if app.rendered_history_generation == app.history_generation {
            return;
        }

        app.history_lines = self
            .history_lines(app)
            .into_iter()
            .map(Self::owned_line)
            .collect();
        app.rendered_history_generation = app.history_generation;
    }

    fn owned_line(line: Line<'_>) -> Line<'static> {
        let mut owned = Line::from(
            line.spans
                .into_iter()
                .map(|span| Span::styled(span.content.to_string(), span.style))
                .collect::<Vec<_>>(),
        );
        owned.style = line.style;
        owned.alignment = line.alignment;
        owned
    }

    fn history_lines(&self, app: &TuiApp) -> Vec<Line<'_>> {
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        for (index, message) in app.messages.iter().enumerate() {
            if index > 0 {
                let previous = &app.messages[index - 1];
                if std::mem::discriminant(previous) != std::mem::discriminant(message)
                    && !matches!(message, TuiMessage::TokenInfo(_))
                {
                    lines.push(Line::from(""));
                }
            }

            lines.extend(self.render_message(message, app.show_thinking, app.show_token_info));
        }

        lines
    }

    pub fn visible_history_lines(
        &self,
        lines: &[Line<'static>],
        width: u16,
        start: usize,
        height: usize,
    ) -> Vec<Line<'static>> {
        let mut visible = Vec::with_capacity(height);
        let mut wrapped_index = 0usize;
        let end = start.saturating_add(height);

        for line in lines {
            if Self::is_user_prompt_line(line) {
                let content_line = Self::strip_user_prompt_prefix(line);
                for wrapped in Self::wrap_line(&content_line, width - 3) {
                    let line = self.format_user_prompt_line(wrapped, width);
                    if Self::push_visible_line(&mut visible, line, &mut wrapped_index, start, end) {
                        return visible;
                    }
                }
            } else {
                for wrapped in Self::wrap_line(line, width) {
                    if Self::push_visible_line(
                        &mut visible,
                        wrapped,
                        &mut wrapped_index,
                        start,
                        end,
                    ) {
                        return visible;
                    }
                }
            }
        }

        visible
    }

    fn strip_user_prompt_prefix(line: &Line<'_>) -> Line<'static> {
        let spans: Vec<_> = if line
            .spans
            .first()
            .is_some_and(|s| s.content == USER_PROMPT_BAR)
        {
            line.spans[1..]
                .iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect()
        } else {
            line.spans
                .iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect()
        };
        let mut result = Line::from(spans);
        result.style = line.style;
        result.alignment = line.alignment;
        result
    }

    fn format_user_prompt_line(&self, mut line: Line<'static>, width: u16) -> Line<'static> {
        let bar_style = self.user_prompt_style().fg(self.theme.user_bar);
        line.spans
            .insert(0, Span::styled(USER_PROMPT_BAR, bar_style));
        let width = width.max(1) as usize;
        let used = line.width();
        if used < width {
            line.spans.push(Span::styled(
                " ".repeat(width - used),
                self.user_prompt_style(),
            ));
        }
        line
    }

    fn push_visible_line(
        visible: &mut Vec<Line<'static>>,
        line: Line<'static>,
        wrapped_index: &mut usize,
        start: usize,
        end: usize,
    ) -> bool {
        if *wrapped_index >= start && *wrapped_index < end {
            visible.push(line);
        }
        *wrapped_index += 1;
        *wrapped_index >= end
    }

    fn wrap_line(line: &Line<'static>, width: u16) -> Vec<Line<'static>> {
        let width = width.max(1) as usize;
        let line_width = line.width();
        if line_width <= width {
            return vec![line.clone()];
        }

        let mut wrapped = Vec::new();
        let mut current_spans = Vec::new();
        let mut current_width = 0usize;

        for span in &line.spans {
            let mut content = String::new();
            for ch in span.content.chars() {
                if current_width == width {
                    wrapped.push(Self::line_from_spans(
                        line,
                        std::mem::take(&mut current_spans),
                    ));
                    current_width = 0;
                }

                content.push(ch);
                current_width += 1;

                if current_width == width {
                    current_spans.push(Span::styled(std::mem::take(&mut content), span.style));
                    wrapped.push(Self::line_from_spans(
                        line,
                        std::mem::take(&mut current_spans),
                    ));
                    current_width = 0;
                }
            }

            if !content.is_empty() {
                current_spans.push(Span::styled(content, span.style));
            }
        }

        if !current_spans.is_empty() || wrapped.is_empty() {
            wrapped.push(Self::line_from_spans(line, current_spans));
        }

        wrapped
    }

    fn line_from_spans(source: &Line<'static>, spans: Vec<Span<'static>>) -> Line<'static> {
        let mut line = Line::from(spans);
        line.style = source.style;
        line.alignment = source.alignment;
        line
    }

    fn user_prompt_bar_line(&self) -> Line<'static> {
        let style = self.user_prompt_style();
        Line::from(vec![Span::styled(
            USER_PROMPT_BAR,
            style.fg(self.theme.user_bar),
        )])
    }

    fn is_user_prompt_line(line: &Line<'_>) -> bool {
        line.spans
            .first()
            .map(|span| span.content == USER_PROMPT_BAR)
            .unwrap_or(false)
    }

    fn user_prompt_style(&self) -> Style {
        Style::default().bg(self.theme.user_box)
    }

    fn help_spec_to_text(&self, spec: Vec<(&str, &str)>) -> Text<'static> {
        let mut spans = Vec::new();
        let style = Style::default().fg(self.theme.header);
        let dim = Style::default().dim();
        spans.push(Span::raw("  "));
        for (i, (label, desc)) in spec.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(label.to_string(), style));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(desc.to_string(), dim));
        }
        Text::from(Line::from(spans))
    }

    fn arguments_json_fields(arguments: &Value, fields: &[&str]) -> String {
        fields
            .iter()
            .filter_map(|&f| {
                arguments.get(f).and_then(|v| {
                    if v.is_string() {
                        v.as_str().map(|s| s.to_string())
                    } else {
                        Some(v.to_string())
                    }
                })
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn input_prompt_lines(&self, input: String, width: u16) -> Vec<Line<'static>> {
        let line = Line::from(vec![
            Span::raw(INPUT_PROMPT_PREFIX),
            Span::raw(input),
            Span::raw(" "),
        ]);
        Self::wrap_line(&line, width)
    }
}
