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
    input::{FileEntry, InputMode, PickList},
    models::ModelPickerRow,
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

struct PickListRenderSpec {
    title: &'static str,
    max_width: u16,
    empty_text: &'static str,
    help_text: &'static str,
}

pub struct Render {}

impl Render {
    pub fn draw(app: &mut TuiApp, frame: &mut Frame<'_>) {
        let area = frame.area();
        let input_width = area.width - 4;
        let input_prompt = Self::input_prompt_lines(app.input.clone(), input_width);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(2 + input_prompt.len() as u16),
            ])
            .split(area);

        frame.render_widget(Clear, area);
        Self::draw_history(app, frame, chunks[0]);
        Self::draw_input(app, frame, chunks[1], input_prompt, input_width as usize);

        match &app.mode {
            InputMode::Command { picker } => {
                Self::draw_commands(frame, chunks[1], picker);
            }
            InputMode::Session { picker } => {
                Self::draw_sessions(frame, chunks[1], picker, false);
            }
            InputMode::SessionRename { picker } => {
                Self::draw_sessions(frame, chunks[1], picker, true);
            }
            InputMode::Models { picker } => {
                Self::draw_models(frame, chunks[1], picker);
            }
            InputMode::Agents { picker } => {
                Self::draw_agents(frame, chunks[1], picker);
            }
            InputMode::Files { picker } => {
                Self::draw_files(frame, chunks[1], picker);
            }
            InputMode::PromptInput | InputMode::Exit => {}
            InputMode::Question {
                title,
                options,
                selected,
            } => {
                Self::draw_question(frame, chunks[1], title, options, *selected);
            }
        }
    }

    fn draw_commands(
        frame: &mut Frame<'_>,
        input_area: Rect,
        picker: &PickList<crate::commands::Command>,
    ) {
        Self::draw_pick_list(
            frame,
            input_area,
            picker,
            PickListRenderSpec {
                title: "Commands",
                max_width: 50,
                empty_text: "No matching commands",
                help_text: "  Type: filter | Enter: select | Esc: close",
            },
            |cmd, row_index, selected_row| {
                let content = format!("/{} - {}", cmd.name, cmd.description);
                if row_index == selected_row {
                    ListItem::new(content).style(Style::default().bg(Color::DarkGray))
                } else {
                    ListItem::new(content)
                }
            },
        );
    }

    fn draw_sessions(
        frame: &mut Frame<'_>,
        input_area: Rect,
        picker: &PickList<crate::session::Session>,
        renaming: bool,
    ) {
        let help_text = if renaming {
            "  Enter: save | Esc: cancel"
        } else {
            "  Enter: load | Ctrl+E: rename | Ctrl+D: delete | Esc: close"
        };
        Self::draw_pick_list(
            frame,
            input_area,
            picker,
            PickListRenderSpec {
                title: "Sessions",
                max_width: 50,
                empty_text: "No sessions",
                help_text,
            },
            |session, row_index, selected_row| {
                let item = ListItem::new(session.display_name());
                if row_index == selected_row {
                    item.style(Style::default().bg(Color::DarkGray))
                } else {
                    item
                }
            },
        );
    }

    fn draw_models(frame: &mut Frame<'_>, input_area: Rect, picker: &PickList<ModelPickerRow>) {
        let display_rows = Self::model_display_rows(picker);
        Self::draw_indexed_pick_list(
            frame,
            input_area,
            picker,
            &display_rows,
            PickListRenderSpec {
                title: "Models",
                max_width: 70,
                empty_text: "No matching models",
                help_text: "  Type: filter | Enter: select | Ctrl+R: reload | Esc: close",
            },
            |row, row_index, selected_row| match row {
                ModelPickerRow::Header(label) => {
                    ListItem::new(label.as_str()).style(Style::default().fg(Color::Yellow))
                }
                ModelPickerRow::Separator => ListItem::new(""),
                ModelPickerRow::Model(model) => {
                    let item = ListItem::new(model.label());
                    if row_index == selected_row {
                        item.style(Style::default().bg(Color::DarkGray))
                    } else {
                        item
                    }
                }
            },
        );
    }

    fn model_display_rows(picker: &PickList<ModelPickerRow>) -> Vec<usize> {
        if picker.filtered.is_empty() {
            return Vec::new();
        }

        let mut display_rows = Vec::new();
        let selected: std::collections::BTreeSet<usize> = picker.filtered.iter().copied().collect();

        for (index, row) in picker.rows.iter().enumerate() {
            match row {
                ModelPickerRow::Model(_) if selected.contains(&index) => display_rows.push(index),
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
                | ModelPickerRow::Model(_) => {}
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
                matches!(row, ModelPickerRow::Model(_))
                    && selected.contains(&(header_index + 1 + offset))
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
                matches!(row, ModelPickerRow::Model(_)) && selected.contains(&index)
            });
        let has_after =
            picker.rows[separator_index + 1..]
                .iter()
                .enumerate()
                .any(|(offset, row)| {
                    matches!(row, ModelPickerRow::Model(_))
                        && selected.contains(&(separator_index + 1 + offset))
                });

        has_before && has_after
    }

    fn draw_agents(
        frame: &mut Frame<'_>,
        input_area: Rect,
        picker: &PickList<crate::agents::AgentDefinition>,
    ) {
        Self::draw_pick_list(
            frame,
            input_area,
            picker,
            PickListRenderSpec {
                title: "Agents",
                max_width: 60,
                empty_text: "No matching agents",
                help_text: "  Type: filter | Enter: select | Esc: close",
            },
            |agent, row_index, selected_row| {
                let item = ListItem::new(agent.name.as_str());
                if row_index == selected_row {
                    item.style(Style::default().bg(Color::DarkGray))
                } else {
                    item
                }
            },
        );
    }

    fn draw_files(frame: &mut Frame<'_>, input_area: Rect, picker: &PickList<FileEntry>) {
        Self::draw_pick_list(
            frame,
            input_area,
            picker,
            PickListRenderSpec {
                title: "Files",
                max_width: 60,
                empty_text: "No matching files",
                help_text: "  Type: filter | Enter: select | Esc: close",
            },
            |file, row_index, selected_row| {
                let item = ListItem::new(file.name.clone());
                if row_index == selected_row {
                    item.style(Style::default().bg(Color::DarkGray))
                } else {
                    item
                }
            },
        );
    }

    fn draw_pick_list<'a, T, F>(
        frame: &mut Frame<'_>,
        input_area: Rect,
        picker: &'a PickList<T>,
        spec: PickListRenderSpec,
        row_item: F,
    ) where
        F: Fn(&'a T, usize, usize) -> ListItem<'a>,
    {
        Self::draw_indexed_pick_list(frame, input_area, picker, &picker.filtered, spec, row_item);
    }

    fn draw_indexed_pick_list<'a, T, F>(
        frame: &mut Frame<'_>,
        input_area: Rect,
        picker: &'a PickList<T>,
        display_rows: &[usize],
        spec: PickListRenderSpec,
        row_item: F,
    ) where
        F: Fn(&'a T, usize, usize) -> ListItem<'a>,
    {
        let row_count = display_rows.len().max(1);
        let visible_rows = (row_count as u16).clamp(1, 10);
        let help_height = 1u16;
        let width = spec
            .max_width
            .min(input_area.width.saturating_sub(4).max(1));
        let height = visible_rows + 2 + help_height;
        let x = input_area.x + 2;
        let y = input_area.y - height;
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
                .map(|row_index| row_item(&picker.rows[*row_index], *row_index, selected_row))
                .collect()
        };

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(spec.title)
                .padding(Padding::horizontal(1)),
        );

        let help_para = Paragraph::new(spec.help_text)
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().fg(Color::Yellow));

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(visible_rows + 2),
                Constraint::Length(help_height),
            ])
            .split(rect);

        frame.render_widget(Clear, rect);
        frame.render_widget(list, chunks[0]);
        frame.render_widget(help_para, chunks[1]);
    }

    fn draw_question(
        frame: &mut Frame<'_>,
        input_area: Rect,
        title: &str,
        options: &[String],
        selected: usize,
    ) {
        let wrap_width: usize = 70usize.saturating_sub(4).max(1);
        let visible_limit = 10u16;

        let mut items = Vec::new();
        // Simple word-wrap helper
        let mut line_buf = String::new();
        for word in title.split_whitespace() {
            if line_buf.is_empty() {
                line_buf.push_str(word);
            } else if line_buf.len() + 1 + word.len() <= wrap_width {
                line_buf.push(' ');
                line_buf.push_str(word);
            } else {
                items.push(ListItem::new(std::mem::take(&mut line_buf)));
                line_buf = word.to_string();
            }
        }
        if !line_buf.is_empty() {
            items.push(ListItem::new(line_buf));
        }

        items.push(ListItem::new(""));

        if !options.is_empty() {
            for (i, opt) in options.iter().enumerate() {
                let item = ListItem::new(format!("{}) {}", i + 1, opt));
                if selected == i {
                    items.push(item.style(Style::default().bg(Color::DarkGray)));
                } else {
                    items.push(item);
                }
            }
        }

        let visible_rows = (items.len().max(1) as u16).min(visible_limit);
        let help_height = 1u16;
        let height = visible_rows + 2 + help_height;

        let max_width: u16 = 70;
        let width = max_width.min(input_area.width.saturating_sub(4).max(1));
        let x = input_area.x + 2;
        let y = input_area.y.saturating_sub(height);
        let rect = Rect::new(x, y, width, height);

        let content_list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Question")
                    .padding(Padding::horizontal(1))
                    .style(Style::default().bg(Color::Rgb(90, 0, 0))),
            )
            .style(Style::default().fg(Color::White));

        let help_para =
            Paragraph::new("  Type: add response | Enter: send | Up/Down: select | Tab: cancel")
                .block(Block::default().borders(Borders::NONE))
                .style(Style::default().fg(Color::Yellow));

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(visible_rows + 2),
                Constraint::Length(help_height),
            ])
            .split(rect);

        frame.render_widget(Clear, rect);
        frame.render_widget(content_list, chunks[0]);
        frame.render_widget(help_para, chunks[1]);
    }

    fn draw_history(app: &mut TuiApp, frame: &mut Frame<'_>, area: Rect) {
        let text_width = area.width.saturating_sub(4).max(1);
        let text_height = area.height.saturating_sub(2).max(1);
        app.history_page_size = text_height;

        Self::sync_history_lines(app);

        let wrapped_height = wrapped_line_count(&app.history_lines, text_width);
        let max_scroll = wrapped_height.saturating_sub(text_height);
        let clamped_scroll = app.history_scroll.min(max_scroll);
        let start = max_scroll.saturating_sub(clamped_scroll);
        let lines = Self::visible_history_lines(
            &app.history_lines,
            text_width,
            start as usize,
            text_height as usize,
        );

        let title = if let Some(tokens) = app.context_tokens {
            format!(
                " Gaius - {} - {} | Context: {} ",
                app.model, app.agent_name, tokens
            )
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

    fn sync_history_lines(app: &mut TuiApp) {
        if app.rendered_history_generation == app.history_generation {
            return;
        }

        app.history_lines = Self::history_lines(app)
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

    fn history_lines(app: &TuiApp) -> Vec<Line<'_>> {
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        for (index, message) in app.messages.iter().enumerate() {
            if index > 0 {
                let previous = &app.messages[index - 1];
                if std::mem::discriminant(previous) != std::mem::discriminant(message) {
                    lines.push(Line::from(""));
                }
            }

            lines.extend(Self::render_message(message, app.show_thinking));
        }

        lines
    }

    pub fn visible_history_lines(
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
                    let line = Self::format_user_prompt_line(wrapped, width);
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
        let spans: Vec<_> = if line.spans.first().is_some_and(|s| s.content == "\u{2503} ") {
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

    fn format_user_prompt_line(mut line: Line<'static>, width: u16) -> Line<'static> {
        let bar_style = Self::user_prompt_style().fg(Color::Magenta);
        line.spans.insert(0, Span::styled("\u{2503} ", bar_style));
        let width = width.max(1) as usize;
        let used = line.width();
        if used < width {
            line.spans.push(Span::styled(
                " ".repeat(width - used),
                Self::user_prompt_style(),
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

    pub fn render_message(msg: &TuiMessage, show_thinking: bool) -> Vec<Line<'static>> {
        match msg {
            TuiMessage::Thinking(text) => {
                if !show_thinking {
                    return vec![
                        Line::from(format!("Thinking... {}", text.len()))
                            .style(Style::default().fg(Color::LightBlue)),
                    ];
                }
                let style = Style::default()
                    .fg(Color::LightBlue)
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
                let style = Self::user_prompt_style();
                vec![
                    Self::user_prompt_bar_line(),
                    Line::from(vec![
                        Span::styled("\u{2503} ", style.fg(Color::Magenta)),
                        Span::raw(text.clone()).style(style.italic().bold()),
                    ]),
                    Self::user_prompt_bar_line(),
                ]
            }
            TuiMessage::ToolCall {
                name,
                arguments,
                result,
                error,
            } => {
                let style = Style::default().fg(Color::Cyan);
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
                    "plan" => "".to_string(),
                    _ => "".to_string(),
                };

                let spans = vec![
                    Span::styled(name.clone(), style.add_modifier(Modifier::BOLD)),
                    Span::raw(" "),
                    Span::styled(display, style.add_modifier(Modifier::ITALIC)),
                ];
                let mut ret = vec![Line::from(spans)];
                if *error {
                    let e_style = Style::default().fg(Color::Red);
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
                let style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
                vec![Line::from(text.clone()).style(style)]
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

    fn user_prompt_bar_line() -> Line<'static> {
        let style = Self::user_prompt_style();
        Line::from(vec![Span::styled("\u{2503} ", style.fg(Color::Magenta))])
    }

    fn is_user_prompt_line(line: &Line<'_>) -> bool {
        line.spans
            .first()
            .map(|span| span.content == "\u{2503} ")
            .unwrap_or(false)
    }

    fn user_prompt_style() -> Style {
        Style::default().bg(Color::Rgb(64, 64, 64))
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

    fn draw_input(
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
                    .style(Style::default().bg(Color::Rgb(64, 0, 64)))
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

    fn input_prompt_lines(input: String, width: u16) -> Vec<Line<'static>> {
        let line = Line::from(vec![
            Span::styled(INPUT_PROMPT_PREFIX, Style::default().fg(Color::Green)),
            Span::raw(input),
            Span::raw(" "),
        ]);
        Self::wrap_line(&line, width)
    }
}
