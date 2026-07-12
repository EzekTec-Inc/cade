use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

/// The result of recording a tool call in the `DoomLoopDetector`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagnationResult {
    /// The tool call is safe and within normal repetition limits.
    Ok,
    /// Stagnation detected! The agent is stuck in an infinite loop repeating this tool call.
    Stagnated {
        tool_name: String,
        repeat_count: usize,
        intervention_message: String,
    },
}

/// Pure state-machine that tracks rolling fingerprints of tool calls
/// and detects when the agent is stuck in an infinite "doom-loop".
pub struct DoomLoopDetector {
    fingerprints: VecDeque<u64>,
    window_size: usize,
    repeat_threshold: usize,
}

impl DoomLoopDetector {
    /// Create a new `DoomLoopDetector` with custom thresholds.
    pub fn new(window_size: usize, repeat_threshold: usize) -> Self {
        Self {
            fingerprints: VecDeque::with_capacity(window_size),
            window_size,
            repeat_threshold,
        }
    }

    /// Create a standard `DoomLoopDetector` with default thresholds:
    /// - Sliding Window Size: 4 tool calls
    /// - Repeat Threshold: 3 repeats
    pub fn default() -> Self {
        Self::new(4, 3)
    }

    /// Record a tool call and check if stagnation is triggered.
    /// If stagnation is detected, returns `StagnationResult::Stagnated` with
    /// the pre-formatted system intervention prompt.
    pub fn record_call(&mut self, tool_name: &str, args: &serde_json::Value) -> StagnationResult {
        // Calculate stable fingerprint of the tool call
        let mut h = std::collections::hash_map::DefaultHasher::new();
        tool_name.hash(&mut h);
        args.to_string().hash(&mut h);
        let fp = h.finish();

        // Push to rolling window queue
        self.fingerprints.push_back(fp);
        if self.fingerprints.len() > self.window_size {
            self.fingerprints.pop_front();
        }

        // Count repetitions of this fingerprint in the active window
        let repeat_count = self.fingerprints.iter().filter(|&&x| x == fp).count();

        if repeat_count >= self.repeat_threshold {
            let intervention_message = format!(
                "SYSTEM INTERVENTION: Stagnation detected. You have called '{}' with identical arguments {} times in the last {} iterations. You are stuck in a doom-loop. Do NOT repeat this call. You MUST immediately call `update_memory(label='active_goal', value=...)` to explicitly rewrite your strategy and outline a new approach in your Core memory, or call the `finish` tool with status='blocked' if you are unable to proceed.",
                tool_name, repeat_count, self.window_size
            );

            StagnationResult::Stagnated {
                tool_name: tool_name.to_string(),
                repeat_count,
                intervention_message,
            }
        } else {
            StagnationResult::Ok
        }
    }

    /// Clear all recorded fingerprints (e.g., when resetting state).
    pub fn clear(&mut self) {
        self.fingerprints.clear();
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_doom_loop_stagnation_detection() {
        // -- Setup & Fixtures
        let mut detector = DoomLoopDetector::new(4, 3);
        let tool = "bash";
        let args = json!({ "command": "ls" });
        let other_args = json!({ "command": "pwd" });

        // -- Exec & Check
        // First call: Ok
        assert_eq!(detector.record_call(tool, &args), StagnationResult::Ok);

        // Second call with same arguments: Ok
        assert_eq!(detector.record_call(tool, &args), StagnationResult::Ok);

        // Third call with same arguments: Stagnated!
        if let StagnationResult::Stagnated {
            tool_name,
            repeat_count,
            intervention_message,
        } = detector.record_call(tool, &args)
        {
            assert_eq!(tool_name, "bash");
            assert_eq!(repeat_count, 3);
            assert!(intervention_message.contains("SYSTEM INTERVENTION"));
        } else {
            panic!("Expected stagnation detection on 3rd identical call!");
        }

        // Fourth call with different arguments: Ok
        assert_eq!(
            detector.record_call(tool, &other_args),
            StagnationResult::Ok
        );

        // Clear resets the sliding window
        detector.clear();
        assert_eq!(detector.record_call(tool, &args), StagnationResult::Ok);
    }
}
