//! C-ABI Foreign Function Interface (FFI) for Python and TypeScript / Node.js native bindings.
#![allow(clippy::not_unsafe_ptr_arg_deref)]
#![allow(unsafe_code)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::Arc;

use tokio::runtime::Runtime;

use crate::embedded::EmbeddedSession;
use crate::team::TeamSession;
use cade_agent::team::TeamMode;

pub struct EmbeddedSessionHandle {
    pub session: EmbeddedSession,
    pub rt: Arc<Runtime>,
}

pub struct TeamSessionHandle {
    pub team: TeamSession,
    pub rt: Arc<Runtime>,
}

fn c_str_to_opt_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        None
    } else {
        unsafe { CStr::from_ptr(ptr) }
            .to_str()
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

fn string_to_c_char(s: impl Into<String>) -> *mut c_char {
    let cs = CString::new(s.into()).unwrap_or_default();
    cs.into_raw()
}

/// Free a string allocated by CADE FFI.
#[unsafe(no_mangle)]
pub extern "C" fn cade_string_free(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            let _ = CString::from_raw(s);
        }
    }
}

/// Create an in-process [`EmbeddedSession`].
///
/// `db_path`: path to SQLite db or `NULL` for in-memory `:memory:`.
/// `model`: model ID or `NULL` for default.
/// `system_prompt`: system prompt or `NULL`.
#[unsafe(no_mangle)]
pub extern "C" fn cade_embedded_session_create(
    db_path: *const c_char,
    model: *const c_char,
    system_prompt: *const c_char,
) -> *mut EmbeddedSessionHandle {
    let rt = match Runtime::new() {
        Ok(r) => Arc::new(r),
        Err(_) => return std::ptr::null_mut(),
    };

    let db_opt = c_str_to_opt_string(db_path);
    let model_opt = c_str_to_opt_string(model).unwrap_or_else(|| "anthropic/claude-sonnet-4-5".to_string());
    let sys_opt = c_str_to_opt_string(system_prompt);

    let rt_c = rt.clone();
    let session_res = rt.block_on(async move {
        let mut builder = EmbeddedSession::builder().model(model_opt);
        if let Some(db) = db_opt {
            builder = builder.db_path(db);
        } else {
            builder = builder.in_memory();
        }
        if let Some(sys) = sys_opt {
            builder = builder.system_prompt(sys);
        }
        builder.build().await
    });

    match session_res {
        Ok(session) => Box::into_raw(Box::new(EmbeddedSessionHandle { session, rt: rt_c })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Prompt the embedded agent and return the response string.
#[unsafe(no_mangle)]
pub extern "C" fn cade_embedded_session_prompt(
    handle: *mut EmbeddedSessionHandle,
    prompt: *const c_char,
) -> *mut c_char {
    if handle.is_null() || prompt.is_null() {
        return string_to_c_char("Error: null pointer passed to prompt");
    }

    let h = unsafe { &*handle };
    let prompt_str = match unsafe { CStr::from_ptr(prompt) }.to_str() {
        Ok(s) => s,
        Err(e) => return string_to_c_char(format!("Error: invalid UTF-8 prompt: {e}")),
    };

    let result = h.rt.block_on(async {
        h.session.prompt(prompt_str).await
    });

    match result {
        Ok(ans) => string_to_c_char(ans),
        Err(e) => string_to_c_char(format!("Error: {e}")),
    }
}

/// Set a memory block value in the embedded session. Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn cade_embedded_session_set_memory(
    handle: *mut EmbeddedSessionHandle,
    label: *const c_char,
    value: *const c_char,
) -> i32 {
    if handle.is_null() || label.is_null() || value.is_null() {
        return -1;
    }

    let h = unsafe { &*handle };
    let label_str = match unsafe { CStr::from_ptr(label) }.to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let value_str = match unsafe { CStr::from_ptr(value) }.to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let result = h.rt.block_on(async {
        h.session.set_memory(label_str, value_str).await
    });

    if result.is_ok() { 0 } else { -1 }
}

/// Get a memory block value from the embedded session.
#[unsafe(no_mangle)]
pub extern "C" fn cade_embedded_session_get_memory(
    handle: *mut EmbeddedSessionHandle,
    label: *const c_char,
) -> *mut c_char {
    if handle.is_null() || label.is_null() {
        return std::ptr::null_mut();
    }

    let h = unsafe { &*handle };
    let label_str = match unsafe { CStr::from_ptr(label) }.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let result = h.rt.block_on(async {
        h.session.get_memory(label_str).await
    });

    match result {
        Ok(Some(v)) => string_to_c_char(v),
        _ => std::ptr::null_mut(),
    }
}

/// Free an [`EmbeddedSessionHandle`].
#[unsafe(no_mangle)]
pub extern "C" fn cade_embedded_session_free(handle: *mut EmbeddedSessionHandle) {
    if !handle.is_null() {
        unsafe {
            let _ = Box::from_raw(handle);
        }
    }
}

/// Create a [`TeamSession`].
///
/// `mode_id`: 0 = Coordinate, 1 = Route, 2 = Tasks.
#[unsafe(no_mangle)]
pub extern "C" fn cade_team_session_create(
    team_id: *const c_char,
    name: *const c_char,
    mode_id: i32,
) -> *mut TeamSessionHandle {
    let rt = match Runtime::new() {
        Ok(r) => Arc::new(r),
        Err(_) => return std::ptr::null_mut(),
    };

    let tid = c_str_to_opt_string(team_id).unwrap_or_else(|| format!("team-{}", uuid::Uuid::new_v4()));
    let tname = c_str_to_opt_string(name).unwrap_or_else(|| "Collaborative Squad".to_string());
    let mode = match mode_id {
        1 => TeamMode::Route,
        2 => TeamMode::Tasks,
        _ => TeamMode::Coordinate,
    };

    let rt_c = rt.clone();
    let team_res = rt.block_on(async move {
        TeamSession::builder()
            .team_id(tid)
            .name(tname)
            .mode(mode)
            .in_memory()
            .build()
            .await
    });

    match team_res {
        Ok(team) => Box::into_raw(Box::new(TeamSessionHandle { team, rt: rt_c })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Run a mission across the team squad. Returns a JSON array string of squad results.
#[unsafe(no_mangle)]
pub extern "C" fn cade_team_session_run(
    handle: *mut TeamSessionHandle,
    prompt: *const c_char,
) -> *mut c_char {
    if handle.is_null() || prompt.is_null() {
        return string_to_c_char("[]");
    }

    let h = unsafe { &*handle };
    let prompt_str = match unsafe { CStr::from_ptr(prompt) }.to_str() {
        Ok(s) => s,
        Err(e) => return string_to_c_char(format!("Error: {e}")),
    };

    let result = h.rt.block_on(async {
        h.team.run(prompt_str).await
    });

    match result {
        Ok(items) => {
            let json_str = serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string());
            string_to_c_char(json_str)
        }
        Err(e) => string_to_c_char(format!("Error: {e}")),
    }
}

/// Free a [`TeamSessionHandle`].
#[unsafe(no_mangle)]
pub extern "C" fn cade_team_session_free(handle: *mut TeamSessionHandle) {
    if !handle.is_null() {
        unsafe {
            let _ = Box::from_raw(handle);
        }
    }
}
