//! Tool orchestration — modeled after Claude Code's `toolOrchestration.ts:91-177`.
//!
//! Partition tool calls into concurrent-safe batches, execute batches with
//! max concurrency control, and propagate sibling abort on Bash errors.

pub use crate::tool_execution::{
    SiblingAbortController, ToolBatch, ToolCall, ToolExecutionArgs, ToolExecutionResult,
};
use researchcode_kernel::tool::{find_tool_spec, ToolRisk};

// ── Tool call model for orchestration ──────────────────────────────────────────

/// Execution state for a single tracked tool in a streaming batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackedToolState {
    Queued,
    Executing,
    Completed,
    Yielded,
    Discarded,
}

#[derive(Debug, Clone)]
pub struct TrackedTool {
    pub tool_call: ToolCall,
    pub state: TrackedToolState,
    pub result: Option<ToolExecutionResult>,
    pub error: Option<String>,
}

/// Partition tools into batches following Claude Code's `partitionToolCalls`
/// (toolOrchestration.ts:91-116).
///
/// Rules:
/// - Consecutive concurrent-safe tools form one batch
/// - Each non-concurrent-safe tool gets its own serial batch
/// - Interactive tools are always serial
/// - Command-executing tools are serial (shell commands can conflict)
pub fn partition_tool_calls(tools: &[ToolCall]) -> Vec<ToolBatch> {
    let mut batches: Vec<ToolBatch> = Vec::new();
    let mut current_batch: Vec<ToolCall> = Vec::new();
    let mut batch_idx = 0;

    for tool in tools {
        if is_concurrent_safe(&tool.tool_id) {
            current_batch.push(tool.clone());
        } else {
            // Flush current concurrent batch — enable sibling abort if it contains shell commands
            let has_shell = current_batch
                .iter()
                .any(|t| t.tool_id == "shell.command" || t.tool_id == "powershell.command");
            if !current_batch.is_empty() {
                batches.push(ToolBatch {
                    batch_id: format!("batch_{batch_idx}"),
                    tools: std::mem::take(&mut current_batch),
                    abort_on_sibling_error: has_shell,
                });
                batch_idx += 1;
            }
            // Serial batch for this tool — enable sibling abort for shell commands
            let is_shell = tool.tool_id == "shell.command" || tool.tool_id == "powershell.command";
            batches.push(ToolBatch {
                batch_id: format!("batch_{batch_idx}"),
                tools: vec![tool.clone()],
                abort_on_sibling_error: is_shell,
            });
            batch_idx += 1;
        }
    }

    // Flush remaining concurrent batch
    if !current_batch.is_empty() {
        batches.push(ToolBatch {
            batch_id: format!("batch_{batch_idx}"),
            tools: current_batch,
            abort_on_sibling_error: false,
        });
    }

    batches
}

// ── Concurrent safety classification ───────────────────────────────────────────

/// A tool is concurrent-safe if it does NOT execute commands and is NOT interactive.
/// (toolOrchestration.ts:9-17 — tools marked with `isConcurrentSafe: true`)
fn is_concurrent_safe(tool_id: &str) -> bool {
    // Explicitly serial tools
    let serial = matches!(
        tool_id,
        "shell.command"
            | "powershell.command"
            | "ask_user"
            | "browser.open"
            | "skill.run"
            | "team.create"
            | "team.delete"
            | "team.message"
            | "task.output"
            | "task.stop"
    );

    if serial {
        return false;
    }

    // Check tool spec risk level
    if let Some(spec) = find_tool_spec(tool_id) {
        match spec.risk {
            ToolRisk::Interactive | ToolRisk::ExecutesCommand => false,
            _ => true,
        }
    } else {
        // Unknown tools default to serial for safety
        false
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(tool_call_id: &str, tool_id: &str) -> ToolCall {
        ToolCall {
            tool_call_id: tool_call_id.to_string(),
            tool_id: tool_id.to_string(),
            args: ToolExecutionArgs::default(),
        }
    }

    #[test]
    fn partitions_consecutive_safe_tools_into_one_batch() {
        let tools = vec![
            make_tool("1", "file.read"),
            make_tool("2", "file.read"),
            make_tool("3", "search.ripgrep"),
        ];
        let batches = partition_tool_calls(&tools);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].tools.len(), 3);
    }

    #[test]
    fn partitions_shell_command_into_own_batch() {
        let tools = vec![
            make_tool("1", "file.read"),
            make_tool("2", "shell.command"),
            make_tool("3", "file.write"),
        ];
        let batches = partition_tool_calls(&tools);
        assert_eq!(batches.len(), 3); // [read, read/write], [shell], [write]
                                      // Actually: [file.read] batch, [shell.command] batch, [file.write] batch
                                      // Wait — file.read and file.write are both concurrent-safe but separated by shell
        assert_eq!(batches[0].tools.len(), 1); // file.read
        assert_eq!(batches[1].tools.len(), 1); // shell.command
        assert_eq!(batches[2].tools.len(), 1); // file.write
    }

    #[test]
    fn concurrent_safe_tools_after_shell_get_own_batch() {
        let tools = vec![
            make_tool("1", "shell.command"),
            make_tool("2", "file.read"),
            make_tool("3", "web.search"),
        ];
        let batches = partition_tool_calls(&tools);
        assert_eq!(batches.len(), 2); // [shell], [file.read, web.search]
        assert_eq!(batches[0].tools.len(), 1); // shell
        assert_eq!(batches[1].tools.len(), 2); // read + search
    }

    #[test]
    fn unknown_tool_is_serial() {
        let tools = vec![make_tool("1", "unknown.tool"), make_tool("2", "file.read")];
        let batches = partition_tool_calls(&tools);
        assert_eq!(batches.len(), 2);
    }

    #[test]
    fn empty_tools_produces_no_batches() {
        let batches = partition_tool_calls(&[]);
        assert!(batches.is_empty());
    }

    #[test]
    fn sibling_abort_propagates() {
        let abort = SiblingAbortController::new();
        assert!(!abort.is_aborted());
        abort.abort();
        assert!(abort.is_aborted());
    }
}
