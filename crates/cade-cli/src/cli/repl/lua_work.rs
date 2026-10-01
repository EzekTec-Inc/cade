//! Bounded Lua host work; no terminal reads and no app lock across awaits.
use super::Repl;
use std::sync::Arc;

impl Repl {
    pub(crate) fn lua_work_pump(&self) -> impl Fn() + Send + Sync + 'static {
        let app = self.app.clone();
        let slots = self.lua_tool_slots.clone();
        let mcp = self.mcp.clone();
        let hooks = self.hooks.clone();
        let stats = self.session_stats.clone();
        let runtime = Arc::new(
            cade_agent::tools::ToolRuntime::new(
                Arc::new(self.client.clone()),
                self.mcp.clone(),
                self.agent_id(),
                self.cwd.clone(),
            )
            .with_conversation(self.conversation_id())
            .with_backend(self.exec_backend.clone()),
        );
        move || {
            // Never wait for the TUI lock in the redraw task, and never remove
            // a request until an execution slot is reserved.
            for _ in 0..4 {
                let Ok(permit) = slots.clone().try_acquire_owned() else {
                    break;
                };
                let work = {
                    let Some(guard) = app.try_lock() else {
                        break;
                    };
                    let Some(lua) = &guard.lua_engine else {
                        break;
                    };
                    let request = lua
                        .tool_queue
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .pop_front();
                    request.map(|request| {
                        (request, lua.ui_event_queue.clone(), lua.work_ready.clone())
                    })
                };
                let Some(((name, args), events, wake)) = work else {
                    break;
                };
                let (mcp, hooks, stats, runtime, app) = (
                    mcp.clone(),
                    hooks.clone(),
                    stats.clone(),
                    runtime.clone(),
                    app.clone(),
                );
                tokio::spawn(async move {
                    let _permit = permit;
                    let id = uuid::Uuid::new_v4().to_string();
                    let result = Self::run_tool_inner(
                        &id, &name, &args, &mcp, &hooks, &app, &runtime, None, None, &stats,
                    )
                    .await;
                    let payload = serde_json::json!({
                        "tool_name": name, "is_error": result.is_error, "content": result.output,
                    });
                    // Backpressure completions without holding the queue lock
                    // across an await. The execution permit bounds waiting tasks.
                    let mut payload = Some(payload);
                    loop {
                        // A reloaded plugin no longer owns this callback queue.
                        // Do not let abandoned completions exhaust host slots.
                        if let Some(guard) = app.try_lock()
                            && !guard
                                .lua_engine
                                .as_ref()
                                .is_some_and(|lua| Arc::ptr_eq(&lua.ui_event_queue, &events))
                        {
                            break;
                        }
                        let queued = {
                            let mut queue = events.lock().unwrap_or_else(|e| e.into_inner());
                            if queue.len() < cade_tui::lua_engine::LUA_QUEUE_LIMIT {
                                queue.push_back(("tool_complete".into(), payload.take().unwrap()));
                                true
                            } else {
                                false
                            }
                        };
                        wake.notify_one();
                        if queued {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(16)).await;
                    }
                });
            }
        }
    }
}
