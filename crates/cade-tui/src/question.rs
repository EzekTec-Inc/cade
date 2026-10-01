//! Question payloads and the standalone adapter to the shared decision dialog.
use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub header: String,
    pub text: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
    pub allow_other: bool,
    pub progress: Option<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionAnswer {
    Single(String),
    /// Free text must never impersonate an authorization option label.
    Custom(String),
    Multi(Vec<String>),
}

impl QuestionAnswer {
    pub fn as_str(&self) -> String {
        match self {
            Self::Single(s) | Self::Custom(s) => s.clone(),
            Self::Multi(v) => v.join(", "),
        }
    }
}

pub struct QuestionWidget;

impl QuestionWidget {
    /// Standalone terminal owner. TuiApp callers use ask_question[_async] so
    /// their conversation and draft remain visible behind the dialog.
    pub fn ask(
        terminal: &mut ratatui::DefaultTerminal,
        question: &Question,
        colors: &crate::colors::ThemeColors,
    ) -> Result<Option<QuestionAnswer>> {
        use crate::overlay_component::{OverlayComponent, OverlayInputResult};
        let mut state = crate::app::ActiveQuestionState {
            approval_id: None,
            draw_state: crate::app::ActiveQuestionDrawState::new(question.clone()),
            tx: None,
            result: None,
        };
        loop {
            terminal.draw(|frame| {
                let area = frame.area();
                state.render_overlay(frame, area, colors);
            })?;
            if crossterm::event::poll(std::time::Duration::from_millis(50))?
                && state.handle_event(&crossterm::event::read()?) == OverlayInputResult::Dismiss
            {
                return Ok(state.result.take().flatten());
            }
        }
    }
}
