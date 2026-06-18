/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    render::Render,
    tools::ToolName,
    tui::{TuiApp, TuiMessage, wrapped_line_count},
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};
use serde_json::{self, Value, from_str};
use tui_markdown::{Options, from_str_with_options};

const USER_PROMPT_BAR: &str = "\u{2503} ";

impl Render {
    pub fn draw_history(&self, app: &mut TuiApp, frame: &mut Frame<'_>, area: Rect) {
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

        let agent_label = if app.plan_mode_on {
            format!("{}/plan", app.agent_name)
        } else {
            app.agent_name.clone()
        };

        let title = match (app.context_tokens, app.model.context_len) {
            (Some(tokens), Some(context_len)) if context_len > 0 => {
                let pct = tokens as f64 / context_len as f64 * 100.0;
                format!(
                    " Gaius - {} - {} - {} ({:.0}%) ",
                    app.model.id, agent_label, tokens, pct
                )
            }
            (Some(tokens), _) => {
                format!(" Gaius - {} - {} - {} ", app.model.id, agent_label, tokens)
            }
            _ => {
                format!(" Gaius - {} - {} ", app.model.id, agent_label)
            }
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
            TuiMessage::AgentMessage(text) | TuiMessage::PlanMessage(text) => {
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
                let tool_name = ToolName::from_name(name.as_str());
                let display = tool_name.map_or_else(String::new, |tool| {
                    Self::arguments_json_fields(&json_args, tool.display_fields())
                });

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
                Self::render_tool_results(tool_name, &json_args, result, &mut ret, style);
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
                let md = Self::plan_to_md(args);
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

    pub fn plan_to_md(args: &Value) -> String {
        let mut md = String::new();
        if let Some(goal) = args.get("goal").and_then(|g| g.as_str()) {
            md.push_str("## Goal\n\n");
            md.push_str(goal);
            md.push_str("\n\n");
        }
        if let Some(context) = args.get("context").and_then(|c| c.as_str()) {
            md.push_str("## Context\n\n");
            md.push_str(context);
            md.push_str("\n\n");
        }
        if let Some(steps) = args.get("steps").and_then(|s| s.as_array()) {
            for (index, step) in steps.iter().enumerate() {
                if let Some(step_text) = step.as_str() {
                    md.push_str(&format!("## Step {}\n\n{}\n\n", index + 1, step_text));
                }
            }
        }
        md
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
