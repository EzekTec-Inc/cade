use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Blocked,
}
impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
    pub fn satisfies_dependency(self) -> bool {
        matches!(self, Self::Completed)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }
}
impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub assignee: Option<String>,
    pub parent_id: Option<String>,
    pub dependencies: Vec<String>,
    pub result: Option<String>,
    pub notes: Vec<String>,
    pub created_at: f64,
}
impl Task {
    pub fn new(title: impl Into<String>, desc: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
            title: title.into(),
            description: desc.into(),
            status: TaskStatus::Pending,
            assignee: None,
            parent_id: None,
            dependencies: vec![],
            result: None,
            notes: vec![],
            created_at: now_secs(),
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskList {
    pub tasks: Vec<Task>,
    pub goal_complete: bool,
    pub completion_summary: Option<String>,
}
impl TaskList {
    pub fn create_task(
        &mut self,
        title: impl Into<String>,
        desc: impl Into<String>,
        assignee: Option<String>,
        parent_id: Option<String>,
        deps: Vec<String>,
    ) -> &Task {
        let mut t = Task::new(title, desc);
        t.assignee = assignee;
        t.parent_id = parent_id;
        t.dependencies = deps;
        self.tasks.push(t);
        self.update_blocked_statuses();
        self.tasks.last().unwrap()
    }
    pub fn get_task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }
    pub fn get_task_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|t| t.id == id)
    }
    pub fn update_task_status(
        &mut self,
        id: &str,
        status: TaskStatus,
        result: Option<String>,
    ) -> Option<&Task> {
        let t = self.tasks.iter_mut().find(|t| t.id == id)?;
        t.status = status;
        if let Some(r) = result {
            t.result = Some(r);
        }
        self.update_blocked_statuses();
        self.get_task(id)
    }
    pub fn add_note(&mut self, id: &str, note: impl Into<String>) -> bool {
        if let Some(t) = self.get_task_mut(id) {
            t.notes.push(note.into());
            true
        } else {
            false
        }
    }
    pub fn all_terminal(&self) -> bool {
        !self.tasks.is_empty() && self.tasks.iter().all(|t| t.status.is_terminal())
    }
    pub fn has_duplicate_title(&self, title: &str) -> Option<&Task> {
        let l = title.to_lowercase();
        self.tasks.iter().find(|t| t.title.to_lowercase() == l)
    }
    #[allow(clippy::collapsible_if)]
    pub fn get_summary_string(&self) -> String {
        if self.tasks.is_empty() {
            return "No tasks created yet.".into();
        }
        let mut c: HashMap<&str, usize> = HashMap::new();
        for t in &self.tasks {
            *c.entry(t.status.as_str()).or_default() += 1;
        }
        let parts: Vec<String> = c.iter().map(|(k, v)| format!("{v} {k}")).collect();
        let mut lines = vec![format!(
            "Tasks ({} total: {}):",
            self.tasks.len(),
            parts.join(", ")
        )];
        for t in &self.tasks {
            let a = t
                .assignee
                .as_deref()
                .map(|a| format!(" (assigned: {a})"))
                .unwrap_or_else(|| " (unassigned)".into());
            lines.push(format!(
                "  [{}] {} - {}{}",
                t.id,
                t.title,
                t.status.as_str().to_uppercase(),
                a
            ));
        }
        if self.goal_complete {
            if let Some(ref s) = self.completion_summary {
                lines.push(format!("\nGoal marked complete: {s}"));
            }
        }
        lines.join("\n")
    }
    fn is_blocked(&self, task: &Task) -> bool {
        if task.dependencies.is_empty() {
            return false;
        }
        for d in &task.dependencies {
            match self.get_task(d) {
                None => return true,
                Some(dep) if !dep.status.satisfies_dependency() => return true,
                _ => {}
            }
        }
        false
    }
    fn has_failed_dependency(&self, task: &Task) -> bool {
        task.dependencies.iter().any(|d| {
            self.get_task(d)
                .is_some_and(|dep| dep.status == TaskStatus::Failed)
        })
    }
    fn update_blocked_statuses(&mut self) {
        let upd: Vec<(usize, TaskStatus, Option<String>)> = self
            .tasks
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                if t.status == TaskStatus::Blocked {
                    if self.has_failed_dependency(t) {
                        Some((
                            i,
                            TaskStatus::Failed,
                            Some("Automatically failed: a dependency failed.".into()),
                        ))
                    } else if !self.is_blocked(t) {
                        Some((i, TaskStatus::Pending, None))
                    } else {
                        None
                    }
                } else if t.status == TaskStatus::Pending && self.is_blocked(t) {
                    Some((i, TaskStatus::Blocked, None))
                } else {
                    None
                }
            })
            .collect();
        for (i, s, r) in upd {
            self.tasks[i].status = s;
            if let Some(r) = r {
                self.tasks[i].result = Some(r);
            }
        }
    }
}
fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
