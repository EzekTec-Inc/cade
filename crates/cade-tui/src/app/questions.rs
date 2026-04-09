//! Interactive question panel — ask_question, ask_question_blocking,
//! ask_question_async, and handle_question_key.

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyModifiers,
};

use crate::Result;

use super::{
    ActiveQuestionDrawState, ActiveQuestionState, RenderLine, TuiApp,
};

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

        let mut cursor_pos: usize = 0;
        let mut custom_text = String::new();
        let mut checked: Vec<bool> = vec![false; n_real];

        // snap to bottom when asking
        self.scroll = 0;

        let answer: Option<crate::question::QuestionAnswer> = 'widget: loop {
            self.active_question = Some(ActiveQuestionState {
                draw_state: ActiveQuestionDrawState {
                    question: question.clone(),
                    cursor_pos,
                    custom_text: custom_text.clone(),
                    checked: checked.clone(),
                    n_real,
                    has_other,
                    has_submit,
                    total_items,
                    other_idx,
                    submit_idx,
                },
                tx: None,
                key_tx: None,
            });

            self.draw()?;

            if !event::poll(std::time::Duration::from_millis(50))? {
                continue;
            }
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match (code, modifiers) {
                    (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        break 'widget None;
                    }
                    (KeyCode::Up, _) => {
                        cursor_pos = cursor_pos.saturating_sub(1);
                    }
                    (KeyCode::Down, _) => {
                        if cursor_pos + 1 < total_items {
                            cursor_pos += 1;
                        }
                    }
                    (KeyCode::Tab, _) => {
                        cursor_pos = (cursor_pos + 1) % total_items;
                    }
                    (KeyCode::BackTab, _) => {
                        cursor_pos = if cursor_pos == 0 {
                            total_items - 1
                        } else {
                            cursor_pos - 1
                        };
                    }
                    (KeyCode::Char(c), KeyModifiers::NONE) if c.is_ascii_digit() && c != '0' => {
                        let idx = (c as usize) - ('0' as usize) - 1;
                        if idx < total_items {
                            if question.multi_select {
                                if idx < n_real {
                                    checked[idx] = !checked[idx];
                                    cursor_pos = idx;
                                }
                            } else if idx != other_idx {
                                let label = question.options[idx].label.clone();
                                break 'widget Some(crate::question::QuestionAnswer::Single(label));
                            } else {
                                cursor_pos = idx;
                            }
                        }
                    }
                    (KeyCode::Backspace, _) if cursor_pos == other_idx => {
                        custom_text.pop();
                    }
                    (KeyCode::Enter, _) => {
                        if question.multi_select {
                            if cursor_pos == submit_idx {
                                let selected: Vec<String> = checked
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, c)| **c)
                                    .map(|(i, _)| question.options[i].label.clone())
                                    .collect();
                                if !selected.is_empty() {
                                    break 'widget Some(crate::question::QuestionAnswer::Multi(
                                        selected,
                                    ));
                                }
                            } else if cursor_pos == other_idx {
                                if !custom_text.is_empty() {
                                    break 'widget Some(crate::question::QuestionAnswer::Multi(
                                        vec![custom_text.clone()],
                                    ));
                                }
                            } else if cursor_pos < n_real {
                                checked[cursor_pos] = !checked[cursor_pos];
                            }
                        } else if cursor_pos == other_idx {
                            if !custom_text.is_empty() {
                                break 'widget Some(crate::question::QuestionAnswer::Single(
                                    custom_text.clone(),
                                ));
                            }
                        } else {
                            let label = question.options[cursor_pos].label.clone();
                            break 'widget Some(crate::question::QuestionAnswer::Single(label));
                        }
                    }
                    (KeyCode::Char(c), m)
                        if cursor_pos == other_idx
                            && (m == KeyModifiers::NONE || m == KeyModifiers::SHIFT) =>
                    {
                        custom_text.push(c);
                    }
                    _ => {}
                }
            }
        };

        self.active_question = None;

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

        let mut cursor_pos: usize = 0;
        let mut custom_text: String = String::new();
        let mut checked: Vec<bool> = vec![false; n_real];

        self.scroll = 0;

        let answer: Option<crate::question::QuestionAnswer> = 'widget: loop {
            // Render with tx = None — tick task will not intercept events.
            self.active_question = Some(ActiveQuestionState {
                draw_state: ActiveQuestionDrawState {
                    question: question.clone(),
                    cursor_pos,
                    custom_text: custom_text.clone(),
                    checked: checked.clone(),
                    n_real,
                    has_other,
                    has_submit,
                    total_items,
                    other_idx,
                    submit_idx,
                },
                tx: None, // ← blocking path: no channel needed
                key_tx: None,
            });

            self.draw()?;

            let key_event = match key_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(k) => k,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break 'widget None,
            };
            let crossterm::event::KeyEvent {
                code, modifiers, ..
            } = key_event;
            match (code, modifiers) {
                (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    break 'widget None;
                }
                (KeyCode::Up, _) => {
                    cursor_pos = cursor_pos.saturating_sub(1);
                }
                (KeyCode::Down, _) => {
                    if cursor_pos + 1 < total_items {
                        cursor_pos += 1;
                    }
                }
                (KeyCode::Tab, _) => {
                    cursor_pos = (cursor_pos + 1) % total_items;
                }
                (KeyCode::BackTab, _) => {
                    cursor_pos = if cursor_pos == 0 {
                        total_items - 1
                    } else {
                        cursor_pos - 1
                    };
                }
                (KeyCode::Char(c), KeyModifiers::NONE) if c.is_ascii_digit() && c != '0' => {
                    let idx = (c as usize) - ('1' as usize);
                    if idx < total_items {
                        if question.multi_select {
                            if idx < n_real {
                                checked[idx] = !checked[idx];
                                cursor_pos = idx;
                            }
                        } else if idx != other_idx {
                            let label = question.options[idx].label.clone();
                            break 'widget Some(crate::question::QuestionAnswer::Single(label));
                        } else {
                            cursor_pos = idx;
                        }
                    }
                }
                (KeyCode::Backspace, _) if cursor_pos == other_idx => {
                    custom_text.pop();
                }
                (KeyCode::Enter, _) => {
                    if question.multi_select {
                        if cursor_pos == submit_idx {
                            let selected: Vec<String> = checked
                                .iter()
                                .enumerate()
                                .filter(|(_, c)| **c)
                                .map(|(i, _)| question.options[i].label.clone())
                                .collect();
                            if !selected.is_empty() {
                                break 'widget Some(crate::question::QuestionAnswer::Multi(
                                    selected,
                                ));
                            }
                        } else if cursor_pos == other_idx {
                            if !custom_text.is_empty() {
                                break 'widget Some(crate::question::QuestionAnswer::Multi(vec![
                                    custom_text.clone(),
                                ]));
                            }
                        } else if cursor_pos < n_real {
                            checked[cursor_pos] = !checked[cursor_pos];
                        }
                    } else if cursor_pos == other_idx {
                        if !custom_text.is_empty() {
                            break 'widget Some(crate::question::QuestionAnswer::Single(
                                custom_text.clone(),
                            ));
                        }
                    } else {
                        let label = question.options[cursor_pos].label.clone();
                        break 'widget Some(crate::question::QuestionAnswer::Single(label));
                    }
                }
                (KeyCode::Char(c), m)
                    if cursor_pos == other_idx
                        && (m == KeyModifiers::NONE || m == KeyModifiers::SHIFT) =>
                {
                    custom_text.push(c);
                }
                _ => {}
            }
        };

        self.active_question = None;

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

        let cursor_pos: usize = 0;
        let custom_text = String::new();
        let checked: Vec<bool> = vec![false; n_real];

        // snap to bottom when asking
        self.scroll = 0;

        let (tx, rx) = tokio::sync::oneshot::channel();

        self.active_question = Some(ActiveQuestionState {
            draw_state: ActiveQuestionDrawState {
                question,
                cursor_pos,
                custom_text,
                checked,
                n_real,
                has_other,
                has_submit,
                total_items,
                other_idx,
                submit_idx,
            },
            tx: Some(tx),
            key_tx: None,
        });

        self.draw()?;
        Ok(rx)
    }

    pub fn handle_question_key(&mut self, k: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut ans_opt: Option<Option<crate::question::QuestionAnswer>> = None;

        if let Some(aq) = &mut self.active_question {
            let st = &mut aq.draw_state;
            match (k.code, k.modifiers) {
                (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    ans_opt = Some(None);
                }
                (KeyCode::Up, _) => {
                    if st.cursor_pos > 0 {
                        st.cursor_pos -= 1;
                    }
                }
                (KeyCode::Down, _) => {
                    if st.cursor_pos + 1 < st.total_items {
                        st.cursor_pos += 1;
                    }
                }
                (KeyCode::Tab, _) => {
                    st.cursor_pos = (st.cursor_pos + 1) % st.total_items;
                }
                (KeyCode::BackTab, _) => {
                    st.cursor_pos = if st.cursor_pos == 0 {
                        st.total_items - 1
                    } else {
                        st.cursor_pos - 1
                    };
                }
                (KeyCode::Char(c), KeyModifiers::NONE) if c.is_ascii_digit() && c != '0' => {
                    let idx = (c as usize) - ('0' as usize) - 1;
                    if idx < st.total_items {
                        if st.question.multi_select {
                            if idx < st.n_real {
                                st.checked[idx] = !st.checked[idx];
                                st.cursor_pos = idx;
                            }
                        } else if idx != st.other_idx {
                            let label = st.question.options[idx].label.clone();
                            ans_opt = Some(Some(crate::question::QuestionAnswer::Single(label)));
                        } else {
                            st.cursor_pos = idx;
                        }
                    }
                }
                (KeyCode::Backspace, _) if st.cursor_pos == st.other_idx => {
                    st.custom_text.pop();
                }
                (KeyCode::Enter, _) => {
                    if st.question.multi_select {
                        if st.cursor_pos == st.submit_idx {
                            let selected: Vec<String> = st
                                .checked
                                .iter()
                                .enumerate()
                                .filter(|(_, c)| **c)
                                .map(|(i, _)| st.question.options[i].label.clone())
                                .collect();
                            if !selected.is_empty() {
                                ans_opt =
                                    Some(Some(crate::question::QuestionAnswer::Multi(selected)));
                            }
                        } else if st.cursor_pos == st.other_idx {
                            if !st.custom_text.is_empty() {
                                ans_opt = Some(Some(crate::question::QuestionAnswer::Multi(vec![
                                    st.custom_text.clone(),
                                ])));
                            }
                        } else if st.cursor_pos < st.n_real {
                            st.checked[st.cursor_pos] = !st.checked[st.cursor_pos];
                        }
                    } else if st.cursor_pos == st.other_idx {
                        if !st.custom_text.is_empty() {
                            ans_opt = Some(Some(crate::question::QuestionAnswer::Single(
                                st.custom_text.clone(),
                            )));
                        }
                    } else {
                        let label = st.question.options[st.cursor_pos].label.clone();
                        ans_opt = Some(Some(crate::question::QuestionAnswer::Single(label)));
                    }
                }
                (KeyCode::Char(c), m)
                    if st.cursor_pos == st.other_idx
                        && (m == KeyModifiers::NONE || m == KeyModifiers::SHIFT) =>
                {
                    st.custom_text.push(c);
                }
                _ => {}
            }
        }

        if let Some(ans) = ans_opt {
            if let Some(mut aq) = self.active_question.take() {
                if let Some(tx) = aq.tx.take() {
                    let _ = tx.send(ans.clone());
                }
                if let Some(a) = &ans {
                    let _ = self.push(RenderLine::QuestionResult {
                        header: aq.draw_state.question.header.clone(),
                        answer: a.as_str(),
                    });
                } else {
                    let _ = self.draw(); // clear question ui on cancel
                }
            }
        } else {
            let _ = self.draw();
        }
    }
}
