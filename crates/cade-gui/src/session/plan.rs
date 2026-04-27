//! Plan-panel state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Set the plan from a `set_plan` tool call. Replaces any existing plan.
    pub fn set_plan(&mut self, steps: Vec<String>) {
        if let Self::Connected { active_plan, .. } = self {
            if steps.is_empty() {
                *active_plan = None;
            } else {
                *active_plan = Some(PlanState {
                    steps: steps
                        .into_iter()
                        .enumerate()
                        .map(|(i, desc)| PlanStep {
                            id: i + 1,
                            description: desc,
                            is_done: false,
                        })
                        .collect(),
                    is_visible: true,
                });
            }
        }
    }

    /// Mark a plan step as done or not done. `step_id` is 1-based.
    pub fn update_plan_step(&mut self, step_id: usize, done: bool) -> bool {
        if let Self::Connected { active_plan: Some(plan), .. } = self {
            if let Some(step) = plan.steps.iter_mut().find(|s| s.id == step_id) {
                step.is_done = done;
                return true;
            }
        }
        false
    }

    /// Read-only access to the active plan.
    pub fn active_plan(&self) -> Option<&PlanState> {
        if let Self::Connected { active_plan, .. } = self {
            active_plan.as_ref()
        } else {
            None
        }
    }

}
