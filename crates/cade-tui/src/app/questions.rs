//! Question entry points all use the same overlay event driver.
use super::{ActiveQuestionState, RenderLine, TuiApp};
use crate::{
    Result,
    question::{Question, QuestionAnswer},
};
use crossterm::event::{self, Event};

impl TuiApp {
    pub fn ask_question(&mut self, question: &Question) -> Result<Option<QuestionAnswer>> {
        let mut answer = self.ask_question_async(question.clone())?;
        let result = loop {
            self.pump_lua_ui_events();
            if self.draw_dirty {
                self.draw()?;
            }
            match answer.try_recv() {
                Ok(result) => break result,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => break None,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            }
            if event::poll(std::time::Duration::from_millis(50))? {
                let event = event::read()?;
                if !self.dispatch_overlay_event(&event)?.0 {
                    match event {
                        Event::Resize(_, _) => self.handle_resize()?,
                        Event::FocusGained => self.has_focus = true,
                        Event::FocusLost => self.has_focus = false,
                        _ => {}
                    }
                }
            }
        };
        self.record_question_answer(question, result)
    }

    /// Compatibility entry point for callers that already forward keys. The
    /// caller is the sole terminal reader; this function never reads crossterm.
    pub fn ask_question_blocking(
        &mut self,
        question: &Question,
        key_rx: std::sync::mpsc::Receiver<crossterm::event::KeyEvent>,
    ) -> Result<Option<QuestionAnswer>> {
        let mut answer = self.ask_question_async(question.clone())?;
        let result = loop {
            self.pump_lua_ui_events();
            if self.draw_dirty {
                self.draw()?;
            }
            if let Ok(result) = answer.try_recv() {
                break result;
            }
            match key_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(key) => {
                    self.dispatch_overlay_event(&Event::Key(key))?;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    // Cancel through the same path so no stale modal survives.
                    self.dispatch_overlay_event(&Event::Key(crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Esc,
                        crossterm::event::KeyModifiers::NONE,
                    )))?;
                    break None;
                }
            }
        };
        self.record_question_answer(question, result)
    }

    fn record_question_answer(
        &mut self,
        question: &Question,
        answer: Option<QuestionAnswer>,
    ) -> Result<Option<QuestionAnswer>> {
        self.scroll = 0;
        self.pending_lines = 0;
        if let Some(answer) = &answer {
            self.push(RenderLine::QuestionResult {
                header: question.header.clone(),
                answer: answer.as_str(),
            })?;
        } else {
            self.draw()?;
        }
        Ok(answer)
    }

    /// Await the receiver with no app lock held. The host's sole event reader
    /// drives dispatch_overlay_event during idle and active turns alike.
    pub fn ask_question_async(
        &mut self,
        question: Question,
    ) -> Result<tokio::sync::oneshot::Receiver<Option<QuestionAnswer>>> {
        self.scroll = 0;
        self.notify_if_unfocused(
            super::notifier::AttentionCue::QuestionAsked,
            "Question Required",
            &question.text,
        );
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.overlays
            .push(Box::new(ActiveQuestionState::new(question, tx)));
        self.draw()?;
        Ok(rx)
    }
}
