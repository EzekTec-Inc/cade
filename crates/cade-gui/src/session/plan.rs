//! Plan-panel state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Set the plan from a `set_plan` tool call. Replaces any existing plan.
    pub fn set_plan(&mut self, title: String, steps: Vec<String>) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { active_plan, .. } = &mut **session;
            if steps.is_empty() {
                *active_plan = None;
            } else {
                *active_plan = Some(PlanState {
                    title,
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
        if let Self::Connected(session) = self {
            if let Some(plan) = &mut session.active_plan {
                if let Some(step) = plan.steps.iter_mut().find(|s| s.id == step_id) {
                    step.is_done = done;
                    return true;
                }
            }
        }
        false
    }

    pub fn on_plan_update(&mut self, plan: &serde_json::Value) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { active_plan, .. } = &mut **session;
            if plan.is_null() {
                *active_plan = None;
            } else {
                let mut steps = Vec::new();
                if let Some(arr) = plan.get("steps").and_then(|v| v.as_array()) {
                    for s in arr {
                        let id = s.get("id").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let desc = s
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let is_done = s.get("is_done").and_then(|v| v.as_bool()).unwrap_or(false);
                        steps.push(PlanStep {
                            id,
                            description: desc,
                            is_done,
                        });
                    }
                }
                let title = plan
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Tasks")
                    .to_string();
                let is_visible = plan
                    .get("is_visible")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                *active_plan = Some(PlanState {
                    title,
                    steps,
                    is_visible,
                });
            }
        }
    }

    /// Read-only access to the active plan.
    pub fn active_plan(&self) -> Option<&PlanState> {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { active_plan, .. } = &**session;
            active_plan.as_ref()
        } else {
            None
        }
    }
}
