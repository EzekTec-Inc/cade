use super::task::TaskList;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberInteraction { pub member_id: String, pub member_name: String, pub task: String, pub result: Option<String>, pub is_error: bool, pub elapsed_secs: u32 }
#[derive(Debug, Clone, Default)]
pub struct TeamRunContext { pub interactions: Vec<MemberInteraction>, pub shared_state: HashMap<String, serde_json::Value>, pub task_list: TaskList }
impl TeamRunContext {
    pub fn new() -> Self { Self::default() }
    pub fn record_interaction(&mut self, mid: impl Into<String>, mname: impl Into<String>, task: impl Into<String>, result: Option<String>, is_error: bool, elapsed: u32) {
        self.interactions.push(MemberInteraction{member_id:mid.into(),member_name:mname.into(),task:task.into(),result,is_error,elapsed_secs:elapsed});
    }
}
