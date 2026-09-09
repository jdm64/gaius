/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::harness::time_now;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;

pub const USER_PROMPT_BAR: &str = "\u{2503} ";

pub struct RenderUtil;

impl RenderUtil {
    pub fn wrap_line(line: &Line<'static>, width: u16) -> Vec<Line<'static>> {
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

    pub fn duration_line(start_time: u64, style: Style) -> Line<'static> {
        let elapsed = time_now().saturating_sub(start_time);
        Line::from(Span::styled(
            format!("  {}", Self::format_duration(elapsed)),
            style.add_modifier(Modifier::DIM),
        ))
    }

    pub fn format_duration(duration_ms: u64) -> String {
        if duration_ms < 60_000 {
            return format!("⏱ {:.3}s", duration_ms as f64 / 1000.0);
        }

        let total_seconds = duration_ms / 1000;
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;

        if hours > 0 {
            format!("⏱ {:01}:{:02}:{:02}", hours, minutes, seconds)
        } else {
            format!("⏱ {:01}:{:02}", minutes, seconds)
        }
    }

    pub fn plan_to_md(args: &Value) -> String {
        args.get("content")
            .and_then(|g| g.as_str())
            .unwrap_or_default()
            .to_string()
    }
}
