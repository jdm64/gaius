/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::render::util::USER_PROMPT_BAR;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};

#[derive(Clone)]
pub struct ColorTheme {
    name: &'static str,
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
            name: "default",
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

impl ColorTheme {
    pub const NAMES: [&'static str; 3] = ["default", "ocean", "forest"];

    pub fn names() -> &'static [&'static str] {
        &Self::NAMES
    }

    pub fn from_name(name: &str) -> Self {
        match name {
            "ocean" => Self {
                name: "ocean",
                header: Color::LightCyan,
                selected: Color::Blue,
                thinking: Color::Cyan,
                user_bar: Color::Blue,
                user_box: Color::Rgb(0, 32, 64),
                inputbox: Color::Rgb(0, 0, 64),
                toolcall: Color::LightCyan,
                error: Color::Red,
            },
            "forest" => Self {
                name: "forest",
                header: Color::LightGreen,
                selected: Color::Green,
                thinking: Color::LightGreen,
                user_bar: Color::Green,
                user_box: Color::Rgb(0, 48, 0),
                inputbox: Color::Rgb(0, 32, 0),
                toolcall: Color::Yellow,
                error: Color::Red,
            },
            _ => Self::default(),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn userbox_style(&self) -> Style {
        Style::default().bg(self.user_box).italic().bold()
    }

    pub fn userbar_style(&self) -> Style {
        Style::default().bg(self.user_box).fg(self.user_bar)
    }

    pub fn thinking_style(&self) -> Style {
        Style::default()
            .fg(self.thinking)
            .add_modifier(Modifier::ITALIC)
    }

    pub fn toolcall_style(&self) -> Style {
        Style::default().fg(self.toolcall)
    }

    pub fn error_style(&self) -> Style {
        Style::default().fg(self.error)
    }

    pub fn header_style(&self) -> Style {
        Style::default().fg(self.header)
    }

    pub fn inputbox_style(&self) -> Style {
        Style::default().bg(self.inputbox)
    }

    pub fn selected_style(&self, is_selected: bool) -> Style {
        if is_selected {
            Style::default().bg(self.selected)
        } else {
            Style::default()
        }
    }

    pub fn selected_color(&self) -> Color {
        self.selected
    }

    pub fn format_user_prompt_line(&self, mut line: Line<'static>, width: u16) -> Line<'static> {
        let bar_style = self.userbar_style();
        line.spans
            .insert(0, Span::styled(USER_PROMPT_BAR, bar_style));
        let width = width.max(1) as usize;
        let used = line.width();
        if used < width {
            line.spans
                .push(Span::styled(" ".repeat(width - used), self.userbox_style()));
        }
        line
    }

    pub fn user_prompt_bar_line(&self) -> Line<'static> {
        Line::from(vec![Span::styled(USER_PROMPT_BAR, self.userbar_style())])
    }

    pub fn help_spec_to_text(&self, spec: Vec<(&str, &str)>) -> Text<'static> {
        let mut spans = Vec::new();
        let style = self.header_style();
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
}
