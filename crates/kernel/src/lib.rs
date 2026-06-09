//! Product Kernel v0 types without external dependencies.

pub mod context;
pub mod hooks;
pub mod memory;
pub mod message;
pub mod model;
pub mod plan;
pub mod subagent;
pub mod task;
pub mod tool;
pub mod transcript;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    User,
    Agent,
    Runtime,
    Tool,
    Model,
    ResearchWorker,
    Policy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelEvent {
    pub event_id: String,
    pub schema_version: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub sequence: u64,
    pub event_type: String,
    pub actor: Actor,
    pub created_at: String,
    pub payload_json: String,
    pub prev_hash: Option<String>,
    pub hash: String,
}

impl KernelEvent {
    /// Compute a deterministic hash from the event's content fields.
    /// Uses sequence + prev_hash + event_type + payload to build an FNV-1a-style hash.
    pub fn compute_hash(&self) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        // Mix in sequence
        for byte in self.sequence.to_le_bytes() {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x100000001b3);
        }
        // Mix in prev_hash (if present)
        if let Some(ref prev) = self.prev_hash {
            for byte in prev.as_bytes() {
                h ^= u64::from(*byte);
                h = h.wrapping_mul(0x100000001b3);
            }
        }
        // Mix in event_type
        for byte in self.event_type.as_bytes() {
            h ^= u64::from(*byte);
            h = h.wrapping_mul(0x100000001b3);
        }
        // Mix in payload_json
        for byte in self.payload_json.as_bytes() {
            h ^= u64::from(*byte);
            h = h.wrapping_mul(0x100000001b3);
        }
        // Mix in created_at for uniqueness
        for byte in self.created_at.as_bytes() {
            h ^= u64::from(*byte);
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("{h:016x}")
    }

    /// Fill in `self.hash` using `compute_hash`.
    pub fn fill_hash(&mut self) {
        self.hash = self.compute_hash();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionRequestType {
    Command,
    FileWrite,
    Network,
    PackageInstall,
    CloudModel,
    ProtectedPath,
    ArtifactExport,
}

impl PermissionRequestType {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "command" => Some(Self::Command),
            "file_write" => Some(Self::FileWrite),
            "network" => Some(Self::Network),
            "package_install" => Some(Self::PackageInstall),
            "cloud_model" => Some(Self::CloudModel),
            "protected_path" => Some(Self::ProtectedPath),
            "artifact_export" => Some(Self::ArtifactExport),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanApprovalRequest {
    pub plan_approval_id: String,
    pub session_id: String,
    pub plan_id: String,
    pub plan_summary: String,
    pub request_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanApprovalDecisionKind {
    Approve,
    Reject,
    RequestRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    pub permission_id: String,
    pub session_id: String,
    pub request_type: PermissionRequestType,
    pub normalized_summary: String,
    pub request_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecisionKind {
    AllowOnce,
    AllowSession,
    AllowProjectRule,
    Deny,
    Modify,
}

pub fn permission_request_type_allows_plan(value: &str) -> bool {
    PermissionRequestType::parse(value).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_request_type_does_not_include_plan() {
        assert_eq!(PermissionRequestType::parse("plan"), None);
        assert!(!permission_request_type_allows_plan("plan"));
    }
}
