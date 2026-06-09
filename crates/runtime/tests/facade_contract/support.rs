use researchcode_kernel::model::NativeModelFamily;
use researchcode_runtime::runtime_facade::{
    AutonomyMode, RuntimeFacade, RuntimeModelMode, RuntimeSessionHandle, RuntimeSessionSnapshot,
};
use researchcode_runtime::subagent::{SubagentRequest, SubagentType};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

pub struct FacadeFixture {
    _temp: TempDir,
    pub workspace: PathBuf,
    pub artifacts: PathBuf,
    pub facade: RuntimeFacade,
}

impl FacadeFixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("create tempdir");
        let workspace = temp.path().join("workspace");
        let artifacts = temp.path().join("artifacts");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::create_dir_all(&artifacts).expect("create artifacts");
        Self {
            _temp: temp,
            workspace: workspace.clone(),
            artifacts: artifacts.clone(),
            facade: RuntimeFacade::new(workspace, artifacts),
        }
    }

    pub fn start(&self) -> RuntimeSessionHandle {
        self.facade
            .start_session(None, RuntimeModelMode::DeepSeek, AutonomyMode::Conservative)
            .expect("start session")
    }

    pub fn start_with(
        &self,
        model: RuntimeModelMode,
        autonomy: AutonomyMode,
    ) -> RuntimeSessionHandle {
        self.facade
            .start_session(None, model, autonomy)
            .expect("start session")
    }

    pub fn snapshot(&self, session_id: &str) -> RuntimeSessionSnapshot {
        self.facade
            .get_session_snapshot(session_id)
            .expect("get snapshot")
    }

    pub fn write_workspace_file(&self, path: &str, content: &str) {
        let full_path = self.workspace.join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(full_path, content).expect("write workspace file");
    }

    pub fn read_workspace_file(&self, path: &str) -> String {
        fs::read_to_string(self.workspace.join(path)).expect("read workspace file")
    }
}

pub fn event_values(jsonl: &str) -> Vec<Value> {
    jsonl
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("event json"))
        .collect()
}

pub fn event_types(jsonl: &str) -> Vec<String> {
    event_values(jsonl)
        .into_iter()
        .filter_map(|value| {
            value
                .get("event_type")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

pub fn count_event_type(jsonl: &str, event_type: &str) -> usize {
    event_types(jsonl)
        .into_iter()
        .filter(|candidate| candidate == event_type)
        .count()
}

pub fn contains_event_type(jsonl: &str, event_type: &str) -> bool {
    event_types(jsonl)
        .into_iter()
        .any(|candidate| candidate == event_type)
}

pub fn readonly_request(parent_session_id: &str, task: &str) -> SubagentRequest {
    SubagentRequest::readonly(
        parent_session_id,
        SubagentType::Explorer,
        task.to_string(),
        NativeModelFamily::DeepSeek,
    )
}

pub fn worker_request(parent_session_id: &str, task: &str, scope: &str) -> SubagentRequest {
    SubagentRequest {
        agent_type: SubagentType::Worker,
        task: task.to_string(),
        model_family: NativeModelFamily::DeepSeek,
        tool_allowlist: SubagentType::Worker.default_tool_allowlist(),
        write_scope: vec![scope.to_string()],
        worktree_required: true,
        worktree_ready: true,
        context_pack: researchcode_runtime::subagent::ContextPack::new(parent_session_id, task),
    }
}
