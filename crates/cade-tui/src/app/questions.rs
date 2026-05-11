//! Interactive question panel — ask_question, ask_question_blocking,
//! ask_question_async, and handle_question_key.

use crossterm::event::{self, Event};

use crate::Result;

use super::{ActiveQuestionDrawState, ActiveQuestionState, RenderLine, TuiApp};

impl TuiApp {
    // -- Interactive Question

    pub fn ask_question(
        &mut self,
        question: &crate::question::Question,
    ) -> Result<Option<crate::question::QuestionAnswer>> {
        let n_real = question.options.len();
        let has_other = question.allow_other;
        let has_submit = question.multi_select;
        let total_items = n_real + usize::from(has_other) + usize::from(has_submit);

        let other_idx = if has_other { n_real } else { usize::MAX };
        let submit_idx = if has_submit {
            n_real + usize::from(has_other)
        } else {
            usize::MAX
        };

        // snap to bottom when asking
        self.scroll = 0;

        let state = ActiveQuestionState {
            draw_state: ActiveQuestionDrawState {
                question: question.clone(),
                cursor_pos: 0,
                custom_text: String::new(),
                checked: vec![false; n_real],
                n_real,
                has_other,
                has_submit,
                total_items,
                other_idx,
                submit_idx,
            },
            tx: None,
            result: None,
        };

        self.overlays.push(Box::new(state));
        self.draw()?;

        let answer = loop {
            if !event::poll(std::time::Duration::from_millis(50))? {
                continue;
            }
            if let Event::Key(key) = event::read()? {
                if let Some(top) = self.overlays.last_mut()
                    && top.id() == "active_question"
                {
                    let res = top.handle_input(key);
                    if matches!(res, crate::overlay_component::OverlayInputResult::Dismiss) {
                        let mut pop = self.overlays.pop().unwrap();
                        let result = pop
                            .take_result()
                            .and_then(|any| {
                                any.downcast::<Option<crate::question::QuestionAnswer>>()
                                    .ok()
                                    .map(|b| *b)
                            })
                            .flatten();
                        break result;
                    }
                }
                self.draw()?;
            }
        };

        if let Some(ans) = &answer {
            self.push(RenderLine::QuestionResult {
                header: question.header.to_string(),
                answer: ans.as_str(),
            })?;
        } else {
            self.draw()?; // clear question ui on cancel
        }

        Ok(answer)
    }

    /// Blocking question modal — driven by key events forwarded through `key_rx`.
    ///
    /// Safe to call from `tokio::task::spawn_blocking`.  Does NOT poll the
    /// crossterm event queue directly; instead the tick task forwards
    /// `KeyEvent`s via the `SyncSender` half of the channel.  This avoids the
    /// deadlock where the tick task consumes an Esc from the EventStream while
    /// this function is waiting on `event::read()`.
    ///
    /// Sets `active_question.tx = None` so the tick task's spin-wait branch
    /// is never entered for this modal.
    ///
    /// This is the canonical path for `prompt_approval` and `handle_ask_user_question`.
    pub fn ask_question_blocking(
        &mut self,
        question: &crate::question::Question,
        key_rx: std::sync::mpsc::Receiver<crossterm::event::KeyEvent>,
    ) -> Result<Option<crate::question::QuestionAnswer>> {
        let n_real = question.options.len();
        let has_other = question.allow_other;
        let has_submit = question.multi_select;
        let total_items = n_real + usize::from(has_other) + usize::from(has_submit);
        let other_idx = if has_other { n_real } else { usize::MAX };
        let submit_idx = if has_submit {
            n_real + usize::from(has_other)
        } else {
            usize::MAX
        };

        self.scroll = 0;

        let state = ActiveQuestionState {
            draw_state: ActiveQuestionDrawState {
                question: question.clone(),
                cursor_pos: 0,
                custom_text: String::new(),
                checked: vec![false; n_real],
                n_real,
                has_other,
                has_submit,
                total_items,
                other_idx,
                submit_idx,
            },
            tx: None,
            result: None,
        };

        self.overlays.push(Box::new(state));
        self.draw()?;

        let answer = loop {
            let key_event = match key_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(k) => k,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break None,
            };

            if let Some(top) = self.overlays.last_mut()
                && top.id() == "active_question"
            {
                let res = top.handle_input(key_event);
                if matches!(res, crate::overlay_component::OverlayInputResult::Dismiss) {
                    let mut pop = self.overlays.pop().unwrap();
                    let result = pop
                        .take_result()
                        .and_then(|any| {
                            any.downcast::<Option<crate::question::QuestionAnswer>>()
                                .ok()
                                .map(|b| *b)
                        })
                        .flatten();
                    break result;
                }
            }
            self.draw()?;
        };

        // V-01 respects the user's scroll position during normal streaming, but
        // after a blocking modal the user MUST see the tool result and agent
        // response immediately — they just took an explicit action (approved /
        // denied / answered).  Reset scroll unconditionally so subsequent pushes
        // land in the visible viewport rather than below it.
        self.scroll = 0;
        self.pending_lines = 0;

        if let Some(ans) = &answer {
            self.push(RenderLine::QuestionResult {
                header: question.header.clone(),
                answer: ans.as_str(),
            })?;
        } else {
            self.draw()?; // clear overlay on cancel
        }

        Ok(answer)
    }

    /// Async question via oneshot channel.
    ///
    /// ONLY valid when an external event driver (the tick task's spin-wait
    /// loop) is concurrently calling `handle_question_key`.  For tool-call
    /// approval use `ask_question_blocking` via `spawn_blocking` instead.
    #[deprecated(
        note = "Use ask_question_blocking (via spawn_blocking) for prompt_approval. \
                ask_question_async is only safe when the tick-task spin-wait is \
                the sole event driver and no async lock contention can occur."
    )]
    pub fn ask_question_async(
        &mut self,
        question: crate::question::Question,
    ) -> Result<tokio::sync::oneshot::Receiver<Option<crate::question::QuestionAnswer>>> {
        let n_real = question.options.len();
        let has_other = question.allow_other;
        let has_submit = question.multi_select;
        let total_items = n_real + usize::from(has_other) + usize::from(has_submit);

        let other_idx = if has_other { n_real } else { usize::MAX };
        let submit_idx = if has_submit {
            n_real + usize::from(has_other)
        } else {
            usize::MAX
        };

        // snap to bottom when asking
        self.scroll = 0;

        let (tx, rx) = tokio::sync::oneshot::channel();

        let state = ActiveQuestionState {
            draw_state: ActiveQuestionDrawState {
                question,
                cursor_pos: 0,
                custom_text: String::new(),
                checked: vec![false; n_real],
                n_real,
                has_other,
                has_submit,
                total_items,
                other_idx,
                submit_idx,
            },
            tx: Some(tx),
            result: None,
        };

        self.overlays.push(Box::new(state));
        self.draw()?;
        Ok(rx)
    }
}
