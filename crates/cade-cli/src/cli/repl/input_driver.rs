//! The idle terminal owner releases the TUI lock before awaiting input/work.
use super::Repl;
use crate::Result;
use cade_tui::app::input::InputOutcome;
use futures::StreamExt;

/// The production idle and active readers share the same wake/select and
/// deferred-input path. Only their redraw cadence and event consumers differ.
pub(crate) enum DriverWake {
    Tick,
    Work,
    Terminal(std::io::Result<crossterm::event::Event>),
    Closed,
}

pub(crate) struct EventPump<S> {
    stream: S,
    work: std::sync::Arc<tokio::sync::Notify>,
    pending: std::sync::Arc<parking_lot::Mutex<Option<crossterm::event::Event>>>,
    tick: tokio::time::Interval,
    cadence: std::time::Duration,
    pending_retry: Option<tokio::time::Instant>,
}

impl<S> EventPump<S>
where
    S: futures::Stream<Item = std::io::Result<crossterm::event::Event>> + Unpin,
{
    pub(crate) fn idle(
        stream: S,
        work: std::sync::Arc<tokio::sync::Notify>,
        pending: std::sync::Arc<parking_lot::Mutex<Option<crossterm::event::Event>>>,
    ) -> Self {
        Self::new(stream, work, pending, std::time::Duration::from_millis(50))
    }

    pub(crate) fn active(
        stream: S,
        work: std::sync::Arc<tokio::sync::Notify>,
        pending: std::sync::Arc<parking_lot::Mutex<Option<crossterm::event::Event>>>,
    ) -> Self {
        Self::new(stream, work, pending, std::time::Duration::from_millis(16))
    }

    fn new(
        stream: S,
        work: std::sync::Arc<tokio::sync::Notify>,
        pending: std::sync::Arc<parking_lot::Mutex<Option<crossterm::event::Event>>>,
        cadence: std::time::Duration,
    ) -> Self {
        let mut tick = tokio::time::interval(cadence);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Self {
            stream,
            work,
            pending,
            tick,
            cadence,
            pending_retry: None,
        }
    }

    pub(crate) async fn next(&mut self) -> DriverWake {
        let Self {
            stream,
            work,
            pending,
            tick,
            cadence,
            pending_retry,
        } = self;
        tokio::select! {
            _ = work.notified() => DriverWake::Work,
            _ = tick.tick() => DriverWake::Tick,
            event = async {
                let has_pending = pending.lock().is_some();
                if has_pending {
                    // Cancellation before this retry leaves the event in the
                    // shared slot for the next idle/active owner. A persistent
                    // deadline prevents periodic ticks starving the retry.
                    let deadline = *pending_retry.get_or_insert_with(|| tokio::time::Instant::now() + *cadence);
                    tokio::time::sleep_until(deadline).await;
                    *pending_retry = None;
                    pending.lock().take().map(Ok)
                } else {
                    *pending_retry = None;
                    stream.next().await
                }
            } => match event {
                Some(event) => DriverWake::Terminal(event),
                None => DriverWake::Closed,
            },
        }
    }

    pub(crate) fn defer(&mut self, event: crossterm::event::Event) {
        let previous = self.pending.lock().replace(event);
        debug_assert!(
            previous.is_none(),
            "one event owner retains at most one input event"
        );
        self.pending_retry = Some(tokio::time::Instant::now() + self.cadence);
    }
}

/// The terminal has exactly one event reader, including across modal waits and
/// active/idle handoff. Aborting the active task releases ownership via Drop.
pub(crate) struct TerminalInputGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl TerminalInputGuard {
    pub(crate) fn claim(flag: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Result<Self> {
        flag.compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .map_err(|_| crate::error::Error::custom("Terminal input already has an event owner"))?;
        Ok(Self(flag))
    }
}

impl Drop for TerminalInputGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Repl {
    pub(crate) async fn read_idle_input(
        &self,
        history: &mut [String],
        hist_idx: &mut Option<usize>,
    ) -> Result<InputOutcome> {
        self.drive_idle_input(history, hist_idx, true).await
    }

    async fn drive_idle_input(
        &self,
        history: &mut [String],
        hist_idx: &mut Option<usize>,
        allow_commands: bool,
    ) -> Result<InputOutcome> {
        let _owner = TerminalInputGuard::claim(self.terminal_driver_active.clone())?;
        let pump = self.lua_work_pump();
        let wake = self
            .app
            .lock()
            .lua_engine
            .as_ref()
            .map(|lua| lua.work_ready.clone())
            .unwrap_or_else(|| std::sync::Arc::new(tokio::sync::Notify::new()));
        let mut reader = EventPump::idle(
            crossterm::event::EventStream::new(),
            wake,
            self.pending_terminal_event.clone(),
        );
        self.app.lock().draw()?;
        loop {
            match reader.next().await {
                DriverWake::Work | DriverWake::Tick => {}
                DriverWake::Terminal(event) => {
                    match event {
                        Ok(event) => {
                            // No input is discarded on try_lock failure. Waiting
                            // for this short dispatch is outside the terminal poll.
                            let result = self
                                .app
                                .lock()
                                .handle_idle_event(event, history, hist_idx)?;
                            if let Some(result) = result {
                                return Ok(result);
                            }
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                DriverWake::Closed => return Ok(InputOutcome::Exit),
            }
            pump();
            let mut app = self.app.lock();
            if let Some(result) = app.tick_idle(allow_commands) {
                return Ok(result);
            }
            if app.draw_dirty || app.signals.any_dirty() || app.is_animating() {
                app.draw()?;
            }
        }
    }

    /// Use the existing active reader when present; otherwise own an idle
    /// reader for the duration of the modal, continuing Lua work and callbacks.
    pub(crate) async fn ask_repl_question(
        &self,
        question: cade_tui::question::Question,
    ) -> Result<Option<cade_tui::question::QuestionAnswer>> {
        let receiver = { self.app.lock().ask_question_async(question.clone())? };
        let answer = if self
            .terminal_driver_active
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            receiver.await.ok().flatten()
        } else {
            let mut history = Vec::new();
            let mut index = None;
            tokio::select! {
                answer = receiver => answer.ok().flatten(),
                result = self.drive_idle_input(&mut history, &mut index, false) => {
                    result?;
                    // Terminal EOF cancels the pending decision through its
                    // normal route rather than leaving an orphaned overlay.
                    let _ = self.app.lock().dispatch_overlay_event(&crossterm::event::Event::Key(
                        crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Esc,
                            crossterm::event::KeyModifiers::NONE),
                    ));
                    None
                }
            }
        };
        let mut app = self.app.lock();
        app.scroll_to_bottom();
        if let Some(answer) = &answer {
            app.push(cade_tui::RenderLine::QuestionResult {
                header: question.header,
                answer: answer.as_str(),
            })?;
        } else {
            app.draw()?;
        }
        Ok(answer)
    }
}

#[cfg(test)]
mod candidate6_tests {
    use super::*;

    #[test]
    fn candidate6_terminal_input_owner_is_exclusive_and_released_on_drop() {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let idle = TerminalInputGuard::claim(flag.clone()).unwrap();
        assert!(TerminalInputGuard::claim(flag.clone()).is_err());
        drop(idle);
        let active = TerminalInputGuard::claim(flag.clone()).unwrap();
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
        drop(active);
        assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn candidate6_idle_and_active_pumps_wake_lua_callbacks_without_terminal_input() {
        use cade_tui::lua_engine::{LUA_UI_BATCH_SIZE, LuaEngine};
        for active in [false, true] {
            let engine = LuaEngine::new().unwrap();
            engine
                .lua
                .load(
                    r#"
                CADE.bind_key("C-q", function() CADE.execute_slash_command("/help") end)
                CADE.bind_ui_callback("tool_complete", function(args)
                    CADE_UI.footer = args.content
                end)
            "#,
                )
                .exec()
                .unwrap();
            let pending = std::sync::Arc::new(parking_lot::Mutex::new(None));
            // Both constructors are used by the corresponding production
            // reader. Only the terminal transport is injected here.
            let stream = futures::stream::pending::<std::io::Result<crossterm::event::Event>>();
            let mut pump = if active {
                EventPump::active(stream, engine.work_ready.clone(), pending)
            } else {
                EventPump::idle(stream, engine.work_ready.clone(), pending)
            };
            assert!(engine.handle_keybinding("C-q"));
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    match pump.next().await {
                        DriverWake::Work => break,
                        DriverWake::Tick => {}
                        _ => panic!("work wakeup must not fabricate a terminal submission"),
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(
                engine.command_queue.lock().unwrap().pop_front().as_deref(),
                Some("/help")
            );

            // The native worker's real completion queue/Notify contract wakes
            // either reader, then the same bounded callback pump updates Lua.
            for index in 0..=LUA_UI_BATCH_SIZE {
                engine.ui_event_queue.lock().unwrap().push_back((
                    "tool_complete".into(),
                    serde_json::json!({"content": format!("result {index}")}),
                ));
            }
            engine.work_ready.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    match pump.next().await {
                        DriverWake::Work => break,
                        DriverWake::Tick => {}
                        _ => panic!("completion must wake without a keyboard event"),
                    }
                }
            })
            .await
            .unwrap();
            assert!(engine.pump_ui_events());
            assert_eq!(
                engine.get_footer_text(),
                Some(format!("result {}", LUA_UI_BATCH_SIZE - 1))
            );
            assert_eq!(engine.ui_event_queue.lock().unwrap().len(), 1);
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    if matches!(pump.next().await, DriverWake::Tick) {
                        break;
                    }
                }
            })
            .await
            .unwrap();
            assert!(engine.pump_ui_events());
            assert_eq!(
                engine.get_footer_text(),
                Some(format!("result {LUA_UI_BATCH_SIZE}"))
            );
            assert!(engine.command_queue.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn candidate6_deferred_input_survives_work_ticks_and_active_idle_handoff() {
        use crossterm::event::Event;
        let owner = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pending = std::sync::Arc::new(parking_lot::Mutex::new(None));
        let wake = std::sync::Arc::new(tokio::sync::Notify::new());
        let active_owner = TerminalInputGuard::claim(owner.clone()).unwrap();
        let input = Event::Paste("unsent 界 draft".into());
        let stream = futures::stream::iter([Ok(input.clone())]).chain(futures::stream::pending());
        let mut active = EventPump::active(stream, wake.clone(), pending.clone());
        let received = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let DriverWake::Terminal(Ok(event)) = active.next().await {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        active.defer(received);
        wake.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match active.next().await {
                    DriverWake::Work => break,
                    DriverWake::Tick => {}
                    DriverWake::Terminal(Ok(event)) => {
                        assert_eq!(event, input);
                        active.defer(event);
                    }
                    _ => panic!("deferred input must neither disappear nor close the stream"),
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(*pending.lock(), Some(input.clone()));
        drop(active);
        drop(active_owner);
        let _idle_owner = TerminalInputGuard::claim(owner).unwrap();
        let mut idle = EventPump::idle(futures::stream::pending(), wake, pending.clone());
        let received = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let DriverWake::Terminal(Ok(event)) = idle.next().await {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(received, input);
        assert!(pending.lock().is_none());
    }
}
