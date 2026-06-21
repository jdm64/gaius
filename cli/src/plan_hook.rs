/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::harness::{Harness, HarnessEvent};

const CLEAR_OPTION: &str = "Clear Context";
const CLEAR_STOP_OPTION: &str = "Clear Context & Stop";
const KEEP_OPTION: &str = "Keep Context";
const KEEP_STOP_OPTION: &str = "Keep Context & Stop";
const REFINE_OPTION: &str = "Refine Plan";
const HOOK_OPTIONS: [&str; 5] = [
    KEEP_OPTION,
    KEEP_STOP_OPTION,
    CLEAR_OPTION,
    CLEAR_STOP_OPTION,
    REFINE_OPTION,
];
const QUESTION_TITLE: &str = "How to proceed with the plan?";

const SYSTEM_PROMPT: &str =
    "You MUST use the plan tool before editing any files because the user switched to Plan mode.";
const REFINE_PROMPT: &str = "Call the plan tool again making the following changes:";
const IMPLEMENT_PROMPT: &str = "Implement the following plan";
const IMPLEMENT_DETAILS_PROMPT: &str = "Implement the plan with the following changes:";

pub enum PlanHook {
    Clear(String),
    ClearStop(String),
    Keep(String),
    KeepStop(String),
    Refine(String),
}

impl PlanHook {
    fn from_str(val: &str) -> Option<Self> {
        let (option, details) = val.split_once('\n').unwrap_or((val, ""));
        let details = details.trim();

        match option.trim() {
            CLEAR_OPTION => Some(Self::Clear(details.to_string())),
            CLEAR_STOP_OPTION => Some(Self::ClearStop(details.to_string())),
            KEEP_OPTION => Some(Self::Keep(details.to_string())),
            KEEP_STOP_OPTION => Some(Self::KeepStop(details.to_string())),
            REFINE_OPTION => Some(Self::Refine(details.to_string())),
            _ => None,
        }
    }

    fn ask_user() -> HarnessEvent {
        HarnessEvent::AskUser {
            title: QUESTION_TITLE.to_string(),
            options: HOOK_OPTIONS.iter().map(|s| s.to_string()).collect(),
        }
    }

    pub fn sys_prompt() -> String {
        SYSTEM_PROMPT.to_owned()
    }

    fn user_prompt(details: String) -> String {
        if details.is_empty() {
            IMPLEMENT_PROMPT.to_owned()
        } else {
            format!("{} {}", IMPLEMENT_DETAILS_PROMPT, details)
        }
    }

    pub fn run<F>(harness: &mut Harness, mut on_event: F) -> bool
    where
        F: FnMut(HarnessEvent) -> Option<String>,
    {
        let Some(plan_text) = harness.plan_text().take() else {
            return false;
        };

        let Some(answer) = on_event(Self::ask_user()) else {
            return false;
        };

        let mode = Self::from_str(answer.as_str());
        let is_stop = matches!(mode, Some(Self::ClearStop(_)) | Some(Self::KeepStop(_)));
        match mode {
            Some(Self::Clear(details)) | Some(Self::ClearStop(details)) => {
                harness.clear_context();
                harness.send_user_message(Self::user_prompt(details), &mut on_event);
                harness.send_plan_message(plan_text, &mut on_event);
            }
            Some(Self::Keep(details)) | Some(Self::KeepStop(details)) => {
                harness.send_user_message(Self::user_prompt(details), &mut on_event);
            }
            Some(Self::Refine(details)) => {
                if details.is_empty() {
                    return true;
                } else {
                    let prompt = format!("{} {}", REFINE_PROMPT, details);
                    harness.send_user_message(prompt, &mut on_event);
                    return false;
                }
            }
            None => {
                return false;
            }
        }

        harness.set_plan_mode(false);

        is_stop
    }
}
