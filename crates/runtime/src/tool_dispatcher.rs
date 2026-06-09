//! ToolDispatcher scheduling policy.
//!
//! This is intentionally only a scheduler now. Actual execution will plug in
//! later, but the ClaudeCode/OpenCode-inspired safety rule is already fixed:
//! only `concurrency_safe` tools may share a batch.

use researchcode_kernel::tool::find_tool_spec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledToolCall {
    pub tool_call_id: String,
    pub tool_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDispatchBatch {
    pub calls: Vec<ScheduledToolCall>,
    pub may_run_concurrently: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolDispatchError {
    UnknownTool(String),
}

pub fn schedule_tool_calls(
    calls: Vec<ScheduledToolCall>,
) -> Result<Vec<ToolDispatchBatch>, ToolDispatchError> {
    let mut batches = Vec::new();
    let mut concurrent_batch: Vec<ScheduledToolCall> = Vec::new();
    for call in calls {
        let Some(spec) = find_tool_spec(&call.tool_id) else {
            return Err(ToolDispatchError::UnknownTool(call.tool_id));
        };
        if spec.concurrency_safe {
            concurrent_batch.push(call);
            continue;
        }
        if !concurrent_batch.is_empty() {
            batches.push(ToolDispatchBatch {
                calls: std::mem::take(&mut concurrent_batch),
                may_run_concurrently: true,
            });
        }
        batches.push(ToolDispatchBatch {
            calls: vec![call],
            may_run_concurrently: false,
        });
    }
    if !concurrent_batch.is_empty() {
        batches.push(ToolDispatchBatch {
            calls: concurrent_batch,
            may_run_concurrently: true,
        });
    }
    Ok(batches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_read_only_tools_together_and_serializes_writes() {
        let batches = schedule_tool_calls(vec![
            ScheduledToolCall {
                tool_call_id: "1".to_string(),
                tool_id: "file.read".to_string(),
            },
            ScheduledToolCall {
                tool_call_id: "2".to_string(),
                tool_id: "search.ripgrep".to_string(),
            },
            ScheduledToolCall {
                tool_call_id: "3".to_string(),
                tool_id: "patch.apply".to_string(),
            },
            ScheduledToolCall {
                tool_call_id: "4".to_string(),
                tool_id: "git.status".to_string(),
            },
        ])
        .unwrap();
        assert_eq!(batches.len(), 3);
        assert!(batches[0].may_run_concurrently);
        assert_eq!(batches[0].calls.len(), 2);
        assert!(!batches[1].may_run_concurrently);
        assert!(batches[2].may_run_concurrently);
    }

    #[test]
    fn unknown_tool_fails_before_dispatch() {
        assert_eq!(
            schedule_tool_calls(vec![ScheduledToolCall {
                tool_call_id: "1".to_string(),
                tool_id: "unknown.tool".to_string(),
            }]),
            Err(ToolDispatchError::UnknownTool("unknown.tool".to_string()))
        );
    }
}
