/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    diff_view::{DiffLineKind, DiffView},
    render::Render,
    render_layout::LiveTimer,
    render_util::{RenderUtil, USER_PROMPT_BAR},
    selection::RowWrapInfo,
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
use serde_json::{self, Value, from_str};
use tui_markdown::{Options, from_str_with_options};

pub struct DisplayPrefs {
    pub thinking: bool,
    pub token_info: bool,
    pub diff_view: bool,
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
}

impl Render {
    pub fn draw_history(&self, app: &mut TuiApp, frame: &mut Frame<'_>, area: Rect) {
        let text_width = area.width.saturating_sub(2).max(1);
        let text_height = area.height.saturating_sub(2).max(1);
        app.history_page_size = text_height;

        self.sync_history_lines(app, text_width);

        let wrapped_height = app.history_layout.visible_lines.len() as u16;
        let max_scroll = wrapped_height.saturating_sub(text_height);
        let clamped_scroll = Render::update_scroll_state(app, wrapped_height, max_scroll);

        let start = max_scroll.saturating_sub(clamped_scroll);
        let lines = self.visible_cached_history_lines(app, start as usize, text_height as usize);

        let lines = app.selection.highlight(
            lines.0,
            lines.1,
            area,
            text_width,
            text_height,
            self.theme.selected,
        );

        let snapshot = &app.snapshot;
        let agent_label = if snapshot.plan_mode_on {
            format!("{}/plan", snapshot.agent_name)
        } else {
            snapshot.agent_name.clone()
        };

        let model_name = match &snapshot.model.reasoning {
            Some(reasoning) => format!("{}:{}", snapshot.model.id, reasoning),
            None => snapshot.model.id.clone(),
        };

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

        app.history_scroll = clamped_scroll;

        self.render_lines_below(app, frame, area);
    }

    fn update_scroll_state(app: &mut TuiApp, wrapped_height: u16, max_scroll: u16) -> u16 {
        let height_growth = if app.history_scroll != 0 && app.history_height != 0 {
            wrapped_height.saturating_sub(app.history_height)
        } else {
            0
        };
        if height_growth > 0 {
            app.history_scroll = app.history_scroll.saturating_add(height_growth);
        }
        app.history_height = wrapped_height;

        let clamped_scroll = app.history_scroll.min(max_scroll);

        app.new_lines_below = if clamped_scroll == 0 {
            0
        } else if height_growth > 0 {
            app.new_lines_below
                .saturating_add(height_growth)
                .min(clamped_scroll)
        } else {
            app.new_lines_below.min(clamped_scroll)
        };

        clamped_scroll
    }

    fn render_lines_below(&self, app: &mut TuiApp, frame: &mut Frame<'_>, area: Rect) {
        if app.new_lines_below > 0 && area.height >= 3 {
            let text = if app.new_lines_below == 1 {
                "1 new line".to_string()
            } else {
                format!("{} new lines", app.new_lines_below)
            };
            let indicator_area = Rect {
                x: area.x,
                y: area.y + area.height - 1,
                width: area.width,
                height: 1,
            };
            let indicator = Paragraph::new(text).alignment(Alignment::Center).style(
                Style::default()
                    .fg(self.theme.header)
                    .add_modifier(Modifier::DIM),
            );
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
            TuiMessage::AgentMessage(text) | TuiMessage::PlanMessage(text) => {
                let options = Options::default();
                from_str_with_options(text, &options)
                    .lines
                    .into_iter()
                    .map(Self::owned_line)
                    .collect()
            }
            TuiMessage::UserPrompt(text) => {
                let style = self.theme.user_prompt_style();
                vec![
                    self.theme.user_prompt_bar_line(),
                    Line::from(vec![
                        Span::styled(USER_PROMPT_BAR, style.fg(self.theme.user_bar)),
                        Span::raw(text.clone()).style(style.italic().bold()),
                    ]),
                    self.theme.user_prompt_bar_line(),
                ]
            }
            TuiMessage::ToolCall {
                name,
                arguments,
                start_time,
            } => {
                let style = Style::default().fg(self.theme.toolcall);
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
                let style = Style::default().fg(self.theme.toolcall);
                let json_args = Value::Null;
                let tool_name = ToolName::from_name(name.as_str());
                let mut ret = Vec::new();
                if *error {
                    let e_style = Style::default().fg(self.theme.error);
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
                let style = Style::default()
                    .fg(self.theme.error)
                    .add_modifier(Modifier::BOLD);
                vec![Line::from(text.clone()).style(style)]
            }
            TuiMessage::CompactionStart { start_time } => {
                let style = Style::default()
                    .fg(self.theme.header)
                    .add_modifier(Modifier::DIM);
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
                let style = Style::default().fg(self.theme.header).dim();
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
                let style = Style::default().fg(self.theme.header).dim();
                vec![Line::from(text).style(style)]
            }
            TuiMessage::Padding => vec![Line::from("")],
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

    fn sync_history_lines(&self, app: &mut TuiApp, text_width: u16) {
        let Some(dirty_from) = app
            .history_layout
            .update_dirty_from(text_width, &self.theme)
        else {
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
        if app.history_layout.last_width != text_width {
            app.history_height = app.history_layout.visible_lines.len() as u16;
        }
        app.history_layout.last_width = text_width;
        app.history_layout.dirty_from = None;
    }

    fn full_rerender(&self, app: &mut TuiApp, text_width: u16) {
        app.history_layout.reset_lines();

        for index in 0..app.messages.len() {
            self.render_message_at(app, index, text_width);
        }

        app.history_layout
            .append_visual_history(0, text_width, &self.theme);
    }

    fn rerender_last(&self, app: &mut TuiApp, last_idx: usize, text_width: u16) {
        app.history_layout.truncate_last_block();
        self.render_message_at(app, last_idx, text_width);
        app.history_layout.append_visual_history(
            app.history_layout.last_block_start,
            text_width,
            &self.theme,
        );
    }

    fn visible_cached_history_lines(
        &self,
        app: &TuiApp,
        start: usize,
        height: usize,
    ) -> (Vec<Line<'static>>, Vec<RowWrapInfo>) {
        let end = start
            .saturating_add(height)
            .min(app.history_layout.visible_lines.len());
        let lines = app.history_layout.visible_lines[start..end].to_vec();
        let row_info = lines
            .iter()
            .enumerate()
            .map(|(offset, line)| {
                let visual_index = start + offset;
                let source_index = app
                    .history_layout
                    .line_starts
                    .partition_point(|&row_start| row_start <= visual_index)
                    .saturating_sub(1);
                RowWrapInfo::new(line, source_index)
            })
            .collect();
        (lines, row_info)
    }

    fn render_message_at(&self, app: &mut TuiApp, index: usize, text_width: u16) {
        let message = &app.messages[index];
        let block_start = app.history_layout.lines.len();

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
                app.history_layout.lines.push(Line::from(""));
            }
        }

        let content_offset = app.history_layout.lines.len();
        let rendered = self.render_message(message, &app.display_prefs, text_width);

        let live_timer = match message {
            TuiMessage::ToolCall { start_time, .. } if *start_time != 0 => {
                Some((*start_time, Style::default().fg(self.theme.toolcall)))
            }
            TuiMessage::CompactionStart { start_time } if *start_time != 0 => {
                Some((*start_time, Style::default().fg(self.theme.header)))
            }
            _ => None,
        };

        if let Some((start_time, style)) = live_timer
            && !rendered.is_empty()
        {
            app.history_layout.live_timers.push(LiveTimer {
                line_index: content_offset + rendered.len() - 1,
                start_time,
                style,
            });
        }

        app.history_layout
            .lines
            .extend(rendered.into_iter().map(Self::owned_line));
        app.history_layout.last_block_start = block_start;
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
