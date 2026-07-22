/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use genai::Error as GenaiError;
use genai::webc::Error as WebcError;
use reqwest::StatusCode;
use std::error::Error;

pub fn is_rate_limit_error(err: &(dyn Error + 'static)) -> bool {
    let Some(genai_err) = err.downcast_ref::<GenaiError>() else {
        return false;
    };
    match genai_err {
        GenaiError::HttpError { status, body, .. } => {
            *status == StatusCode::TOO_MANY_REQUESTS
                || (status.is_client_error() && body_has_nested_rate_limit(body))
        }
        GenaiError::WebAdapterCall { webc_error, .. }
        | GenaiError::WebModelCall { webc_error, .. } => is_webc_rate_limit(webc_error),
        GenaiError::WebStream {
            error: webc_error, ..
        } => is_rate_limit_error(webc_error.as_ref()),
        _ => false,
    }
}

fn body_has_nested_rate_limit(body: &str) -> bool {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    val.get("error")
        .and_then(|e| e.get("metadata"))
        .and_then(|m| m.get("previous_errors"))
        .and_then(|pe| pe.as_array())
        .is_some_and(|errors| {
            errors.iter().any(|pe| {
                pe.get("code")
                    .and_then(|c| c.as_i64())
                    .is_some_and(|code| code == 429)
            })
        })
}

pub fn is_webc_rate_limit(webc_err: &WebcError) -> bool {
    matches!(
        webc_err,
        WebcError::ResponseFailedStatus { status, .. } if *status == StatusCode::TOO_MANY_REQUESTS
    )
}
