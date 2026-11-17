/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_client_protocol::{
    Error, JsonRpcResponse, Responder,
    v2::schema::{AbsolutePath, Error as SchemaError, SessionId},
};
use serde_json::Value;
use std::path::PathBuf;

pub fn reply<T: JsonRpcResponse>(r: Responder<Value>, resp: T) -> Result<(), Error> {
    r.cast::<T>().respond(resp)
}

pub fn reply_no_session(resp: Responder, id: SessionId) -> Result<(), Error> {
    resp.respond_with_error(SchemaError::session_not_found(id).into())
}

pub fn abs_path(path: Option<AbsolutePath>) -> Result<Option<PathBuf>, String> {
    match path {
        Some(p) if p.0.is_absolute() => Ok(Some(p.0)),
        None => Ok(None),
        _ => Err("Absolute path required".into()),
    }
}
