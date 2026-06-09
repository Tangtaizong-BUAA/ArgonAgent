use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInventoryRecord {
    pub tool_id: String,
    pub ok: bool,
    pub preview: String,
}

pub fn is_tool_inventory_read_only_observation(tool_id: &str) -> bool {
    matches!(
        tool_id,
        "file.list_directory"
            | "file.list_tree"
            | "repo.map"
            | "search.ripgrep"
            | "git.status"
            | "file.read"
    )
}

pub fn is_tool_inventory_gated_attempt(tool_id: &str) -> bool {
    matches!(
        tool_id,
        "file.write"
            | "file.edit"
            | "file.multi_edit"
            | "shell.command"
            | "patch.apply"
            | "plan.enter"
            | "todo.write"
    )
}

pub fn successful_observation_count(records: &[ToolInventoryRecord]) -> usize {
    let mut observed = BTreeSet::<String>::new();
    for record in records {
        if record.ok && is_tool_inventory_read_only_observation(&record.tool_id) {
            observed.insert(record.tool_id.clone());
        }
    }
    observed.len()
}

pub fn gated_attempt_count(records: &[ToolInventoryRecord]) -> usize {
    records
        .iter()
        .filter(|record| !record.ok && is_tool_inventory_gated_attempt(&record.tool_id))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(tool_id: &str, ok: bool) -> ToolInventoryRecord {
        ToolInventoryRecord {
            tool_id: tool_id.to_string(),
            ok,
            preview: format!("{tool_id} preview"),
        }
    }

    #[test]
    fn counts_read_only_observations() {
        let records = vec![record("file.list_directory", true)];
        assert_eq!(successful_observation_count(&records), 1);
    }

    #[test]
    fn counts_gated_attempts() {
        let records = vec![record("file.write", false)];
        assert_eq!(gated_attempt_count(&records), 1);
    }
}
