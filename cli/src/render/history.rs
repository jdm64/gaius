/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    diff_view::{DiffLineKind, DiffView},
    dirs::Dirs,
    render::Render,
    render::layout::LiveTimer,
    render::util::{RenderUtil, USER_PROMPT_BAR},
    tools::ToolName,
    tui::{TuiApp, TuiMessage},
};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};
use serde_json::{self, Value, from_str};
use std::{error::Error, fs};
use tui_markdown::{Options, from_str_with_options};

#[derive(Clone, Serialize, Deserialize)]
pub struct DisplayPrefs {
    pub thinking: bool,
    pub token_info: bool,
    pub diff_view: bool,
}

impl Default for DisplayPrefs {
    fn default() -> Self {
        Self {
            thinking: false,
            token_info: true,
            diff_view: true,
        }
    }
}

impl DisplayPrefs {
    fn toggle(field: &mut bool, label: &str) -> String {
        *field = !*field;
        format!("{} display: {}", label, if *field { "on" } else { "off" })
    }

    pub fn toggle_thinking(&mut self) -> String {
        Self::toggle(&mut self.thinking, "Thinking")
    }

    pub fn toggle_token_info(&mut self) -> String {
        Self::toggle(&mut self.token_info, "Token info")
    }

    pub fn toggle_diff_view(&mut self) -> String {
        Self::toggle(&mut self.diff_view, "Diff view")
    }

    pub fn load() -> Result<Self, Box<dyn Error>> {
        let path = Dirs::display_prefs_file()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&contents).unwrap_or_default())
    }

    pub fn save(&self) -> Result<(), Box<dyn Error>> {
        let path = Dirs::display_prefs_file()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(self)?;
        fs::write(path, contents)?;
        Ok(())
    }
}

impl Render {
    pub fn draw_history(&self, app: &mut TuiApp, frame: &mut Frame<'_>, area: Rect) {
        let width = area.width.saturating_sub(2).max(1);
        let height = area.height.saturating_sub(2).max(1);

        self.sync_history_lines(app, width);

        let lines = app.history.visible_cached_lines(height);
        let lines = app.selection.highlight(
            lines.0,
            lines.1,
            area,
            width,
            height,
            self.theme.selected_color(),
        );

        let snapshot = &app.snapshot;
        let agent_label = if snapshot.plan_mode_on {
            format!("{}/plan", snapshot.agent_name)
        } else {
            snapshot.agent_name.clone()
        };

        let mut model_name = match &snapshot.model.reasoning {
            Some(reasoning) => format!("{}:{}", snapshot.model.id, reasoning),
            None => snapshot.model.id.clone(),
        };
        if model_name.is_empty() {
            model_name = "[no model]".to_string();
        }

        let parts: Vec<String> = [
            Some(format!("Gaius - {} - {}", model_name, agent_label)),
            app.context_tokens
                .map(|tokens| match snapshot.model.context_len {
                    Some(context_len) if context_len > 0 => {
                        let pct = tokens as f64 / context_len as f64 * 100.0;
                        format!(" - {} {:.0}%", tokens, pct)
                    }
                    _ => format!(" - {}", tokens),
                }),
            snapshot.total_cost.map(|cost| format!(" ${:.3}", cost)),
        ]
        .into_iter()
        .flatten()
        .collect();

        let title = format!(" {} ", parts.join(""));

        let history = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::TOP)
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(history, area);

        self.render_lines_below(app, frame, area);
    }

    fn render_lines_below(&self, app: &mut TuiApp, frame: &mut Frame<'_>, area: Rect) {
        if app.history.new_lines > 0 && area.height >= 3 {
            let text = if app.history.new_lines == 1 {
                "1 new line".to_string()
            } else {
                format!("{} new lines", app.history.new_lines)
            };
            let indicator_area = Rect {
                x: area.x,
                y: area.y + area.height - 1,
                width: area.width,
                height: 1,
            };
            let indicator = Paragraph::new(text)
                .alignment(Alignment::Center)
                .style(self.theme.header_style());
            frame.render_widget(indicator, indicator_area);
        }
    }

    pub fn render_message(
        &self,
        msg: &TuiMessage,
        prefs: &DisplayPrefs,
        text_width: u16,
    ) -> Vec<Line<'static>> {
        match msg {
            TuiMessage::Thinking(text) => {
                if !prefs.thinking {
                    return vec![
                        Line::from(format!("Thinking... {}", text.len()))
                            .style(self.theme.thinking_style()),
                    ];
                }
                let style = self.theme.thinking_style();
                Self::render_markdown(text, Some(text_width), Some(style))
            }
            TuiMessage::AgentMessage(text) | TuiMessage::PlanMessage(text) => {
                Self::render_markdown(text, Some(text_width), None)
            }
            TuiMessage::UserPrompt(text) => {
                vec![
                    self.theme.user_prompt_bar_line(),
                    Line::from(vec![
                        Span::styled(USER_PROMPT_BAR, self.theme.userbar_style()),
                        Span::raw(text.clone()).style(self.theme.userbox_style()),
                    ]),
                    self.theme.user_prompt_bar_line(),
                ]
            }
            TuiMessage::ToolCall {
                name,
                arguments,
                start_time,
            } => {
                let style = self.theme.toolcall_style();
                let json_args = from_str::<Value>(arguments).unwrap_or_default();
                let tool_name = ToolName::from_name(name.as_str());
                let display = tool_name.map_or_else(String::new, |tool| {
                    Self::arguments_json_fields(&json_args, tool.display_fields())
                });

                let mut lines = vec![Line::from(vec![
                    Span::styled(name.clone(), style.add_modifier(Modifier::BOLD)),
                    Span::raw(" "),
                    Span::styled(display, style.add_modifier(Modifier::ITALIC)),
                ])];

                if *start_time != 0 {
                    lines.push(RenderUtil::duration_line(*start_time, style));
                }

                lines
            }
            TuiMessage::ToolResult {
                name,
                result,
                error,
            } => {
                let style = self.theme.toolcall_style();
                let json_args = Value::Null;
                let tool_name = ToolName::from_name(name.as_str());
                let mut ret = Vec::new();
                if *error {
                    let e_style = self.theme.error_style();
                    let error_lines: Vec<&str> = result.split('\n').collect();
                    for i in error_lines {
                        if !i.is_empty() {
                            ret.push(Line::from(Span::styled(format!("  {}", i), e_style)));
                        }
                    }
                }
                Self::render_tool_results(tool_name, &json_args, result, &mut ret, style);
                ret
            }
            TuiMessage::SystemMessage(text) => {
                let style = self.theme.error_style().add_modifier(Modifier::BOLD);
                vec![Line::from(text.clone()).style(style)]
            }
            TuiMessage::CompactionStart { start_time } => {
                let style = self.theme.header_style();
                let mut lines = vec![Self::compaction_rule_line(style, text_width)];
                if *start_time != 0 {
                    lines.push(RenderUtil::duration_line(*start_time, style));
                }
                lines
            }
            TuiMessage::TokenInfo(text) => {
                if !prefs.token_info {
                    return vec![];
                }
                let style = self.theme.header_style().dim();
                vec![Line::from(text.clone()).style(style).right_aligned()]
            }
            TuiMessage::DiffView(diff) => {
                if !prefs.diff_view {
                    return vec![];
                }
                Self::render_diff_view(diff)
            }
            TuiMessage::TurnDuration(duration_ms) => {
                let text = RenderUtil::format_duration(*duration_ms);
                let style = self.theme.header_style();
                vec![Line::from(text).style(style)]
            }
            TuiMessage::Padding => vec![Line::from("")],
        }
    }

    fn render_markdown(text: &str, width: Option<u16>, style: Option<Style>) -> Vec<Line<'static>> {
        let mut options = Options::default();
        if let Some(wd) = width {
            options = options.table_width(wd);
        }
        let iter = from_str_with_options(text, &options).lines.into_iter();

        if let Some(st) = style {
            iter.map(|mut line| {
                line.spans = line
                    .spans
                    .into_iter()
                    .map(|span| Span::styled(span.content, st.patch(span.style)))
                    .collect();
                Self::owned_line(line)
            })
            .collect()
        } else {
            iter.map(Self::owned_line).collect()
        }
    }

    fn render_diff_view(diff: &DiffView) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let header_style = Style::default().add_modifier(Modifier::BOLD);
        let context_style = Style::default().add_modifier(Modifier::DIM);
        let delete_style = Style::default().fg(Color::Red);
        let insert_style = Style::default().fg(Color::Green);

        lines.push(Line::from(vec![
            Span::styled("diff ", header_style),
            Span::styled(diff.file_path.clone(), header_style),
        ]));

        for hunk in &diff.hunks {
            lines.push(Line::from(Span::styled(
                format!(
                    "@@ -{},{} +{},{} @@",
                    hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
                ),
                context_style,
            )));

            for diff_line in &hunk.lines {
                let (prefix, style) = match diff_line.kind {
                    DiffLineKind::Context => (" ", context_style),
                    DiffLineKind::Delete => ("-", delete_style),
                    DiffLineKind::Insert => ("+", insert_style),
                };
                lines.push(Line::from(vec![
                    Span::styled(prefix.to_string(), style),
                    Span::styled(diff_line.text.clone(), style),
                ]));
                if diff_line.missing_newline {
                    lines.push(Line::from(Span::styled(
                        "\\ No newline at end of file",
                        context_style,
                    )));
                }
            }
        }

        lines
    }

    fn render_tool_results(
        name: Option<ToolName>,
        args: &Value,
        result: &str,
        lines: &mut Vec<Line>,
        style: Style,
    ) {
        match name {
            Some(ToolName::Question) => {
                let answers = result
                    .split("\n")
                    .map(|l| " - ".to_string() + l)
                    .collect::<Vec<_>>();
                for l in answers {
                    lines.push(Line::from(Span::styled::<String, Style>(l, style)));
                }
            }
            Some(ToolName::Plan) => {
                let md = RenderUtil::plan_to_md(args);
                let rendered_lines = Self::render_markdown(&md, None, None);
                lines.push(Line::raw(" "));
                lines.extend(rendered_lines);
                lines.push(Line::raw(" "));
            }
            _ => {}
        }
    }

    fn sync_history_lines(&self, app: &mut TuiApp, text_width: u16) {
        let Some(dirty_from) = app.history.update_dirty_from(text_width, &self.theme) else {
            return;
        };
        let last_idx = app.messages.len().saturating_sub(1);

        if dirty_from == 0 || dirty_from != last_idx {
            // if dirty_from != last_idx then last_block_start is invalid and
            // a whole rerender must be done. This could be optimized by
            // storing block start for each message but probably not worth it.
            self.full_rerender(app, text_width);
        } else {
            self.rerender_last(app, last_idx, text_width);
        }

        // reset height so scroll doesn't drift
        app.history.update_width(text_width);
    }

    fn full_rerender(&self, app: &mut TuiApp, text_width: u16) {
        app.history.reset_lines();

        for index in 0..app.messages.len() {
            self.render_message_at(app, index, text_width);
        }

        app.history
            .append_visual_history(0, text_width, &self.theme);
    }

    fn rerender_last(&self, app: &mut TuiApp, last_idx: usize, text_width: u16) {
        app.history.truncate_last_block();
        self.render_message_at(app, last_idx, text_width);
        app.history
            .append_visual_history(app.history.block_start, text_width, &self.theme);
    }

    fn render_message_at(&self, app: &mut TuiApp, index: usize, text_width: u16) {
        let message = &app.messages[index];
        let block_start = app.history.lines.len();

        if index > 0 {
            let previous = &app.messages[index - 1];
            let is_padding_edge =
                matches!(previous, TuiMessage::Padding) || matches!(message, TuiMessage::Padding);
            if !is_padding_edge
                && std::mem::discriminant(previous) != std::mem::discriminant(message)
                && !matches!(
                    message,
                    TuiMessage::TokenInfo(_)
                        | TuiMessage::TurnDuration(_)
                        | TuiMessage::ToolResult { .. }
                )
            {
                app.history.lines.push(Line::from(""));
            }
        }

        let content_offset = app.history.lines.len();
        let rendered = self.render_message(message, &app.display_prefs, text_width);

        let live_timer = match message {
            TuiMessage::ToolCall { start_time, .. } if *start_time != 0 => {
                Some((*start_time, self.theme.toolcall_style()))
            }
            TuiMessage::CompactionStart { start_time } if *start_time != 0 => {
                Some((*start_time, self.theme.header_style()))
            }
            _ => None,
        };

        if let Some((start_time, style)) = live_timer
            && !rendered.is_empty()
        {
            app.history.timers.push(LiveTimer {
                line_index: content_offset + rendered.len() - 1,
                start_time,
                style,
            });
        }

        app.history
            .lines
            .extend(rendered.into_iter().map(Self::owned_line));
        app.history.block_start = block_start;
    }

    /// Horizontal rule with the word "Compaction" centered.
    fn compaction_rule_line(style: Style, text_width: u16) -> Line<'static> {
        let text_width = text_width as usize;
        let center = " Compaction ";
        let center_len = center.len();
        // Calculate the width for each side of the rule
        let side_width = text_width.saturating_sub(center_len) / 2;
        let rule = "─".repeat(side_width);
        Line::from(vec![
            Span::styled(rule.clone(), style),
            Span::styled(center.to_string(), style.add_modifier(Modifier::BOLD)),
            Span::styled(rule, style),
        ])
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
}
