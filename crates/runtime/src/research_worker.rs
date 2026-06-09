//! Research Worker sidecar launcher.
//!
//! V0 executes the local Python worker with explicit argv and no shell. The
//! worker remains responsible for producing data profile, privacy report, and
//! reproducibility manifest artifacts.

use crate::session::{AgentSession, SessionError};
use researchcode_kernel::PermissionRequestType;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchPackageInstallRequest {
    pub job_id: String,
    pub packages: Vec<String>,
    pub reason: String,
    pub privacy_class: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResearchPackageInstallPolicy {
    PermissionRequired,
    DenyInvalidPackageName(String),
    DenyEmptyRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchCsvProfileRequest {
    pub job_id: String,
    pub input_csv: PathBuf,
    pub output_dir: PathBuf,
    pub worker_cwd: PathBuf,
    pub limits: ResearchWorkerLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchWorkerLimits {
    pub max_input_bytes: u64,
    pub timeout_seconds: u64,
    pub max_memory_mb: u64,
    pub network_enabled: bool,
    pub package_install_enabled: bool,
}

impl Default for ResearchWorkerLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 10_000_000,
            timeout_seconds: 30,
            max_memory_mb: 512,
            network_enabled: false,
            package_install_enabled: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchWorkerRunResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub profile_path: Option<PathBuf>,
    pub privacy_report_path: Option<PathBuf>,
    pub analysis_script_path: Option<PathBuf>,
    pub report_path: Option<PathBuf>,
    pub notebook_path: Option<PathBuf>,
    pub manifest_path: Option<PathBuf>,
    pub manifest_content_hash: Option<String>,
    pub artifact_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResearchWorkerError {
    InputMissing(PathBuf),
    WorkerMissing(PathBuf),
    ResourceLimitExceeded(String),
    Io(String),
}

pub fn classify_research_package_install(
    request: &ResearchPackageInstallRequest,
) -> ResearchPackageInstallPolicy {
    if request.packages.is_empty() {
        return ResearchPackageInstallPolicy::DenyEmptyRequest;
    }
    for package in &request.packages {
        if !is_safe_package_spec(package) {
            return ResearchPackageInstallPolicy::DenyInvalidPackageName(package.clone());
        }
    }
    ResearchPackageInstallPolicy::PermissionRequired
}

pub fn request_research_package_install_permission(
    session: &mut AgentSession,
    permission_id: impl Into<String>,
    request: &ResearchPackageInstallRequest,
) -> Result<ResearchPackageInstallPolicy, SessionError> {
    let policy = classify_research_package_install(request);
    if policy == ResearchPackageInstallPolicy::PermissionRequired {
        session.request_permission(permission_id, PermissionRequestType::PackageInstall, None)?;
    }
    Ok(policy)
}

pub fn run_csv_profile_sidecar(
    request: &ResearchCsvProfileRequest,
) -> Result<ResearchWorkerRunResult, ResearchWorkerError> {
    if !request.input_csv.exists() {
        return Err(ResearchWorkerError::InputMissing(request.input_csv.clone()));
    }
    if !request.worker_cwd.exists() {
        return Err(ResearchWorkerError::WorkerMissing(
            request.worker_cwd.clone(),
        ));
    }
    if request.limits.network_enabled {
        return Err(ResearchWorkerError::ResourceLimitExceeded(
            "network must be disabled for CSV profiling sidecar".to_string(),
        ));
    }
    if request.limits.package_install_enabled {
        return Err(ResearchWorkerError::ResourceLimitExceeded(
            "package install must be disabled for CSV profiling sidecar".to_string(),
        ));
    }
    let input_size = request
        .input_csv
        .metadata()
        .map_err(|error| ResearchWorkerError::Io(error.to_string()))?
        .len();
    if input_size > request.limits.max_input_bytes {
        return Err(ResearchWorkerError::ResourceLimitExceeded(format!(
            "input exceeds max_input_bytes: {input_size} > {}",
            request.limits.max_input_bytes
        )));
    }
    let output = Command::new("python3")
        .args([
            "-m",
            "research_worker",
            "profile-csv",
            &request.job_id,
            &request.input_csv.to_string_lossy(),
            &request.output_dir.to_string_lossy(),
        ])
        .current_dir(&request.worker_cwd)
        .env(
            "RESEARCHCODE_WORKER_TIMEOUT_SECONDS",
            request.limits.timeout_seconds.to_string(),
        )
        .env(
            "RESEARCHCODE_WORKER_MAX_INPUT_BYTES",
            request.limits.max_input_bytes.to_string(),
        )
        .env(
            "RESEARCHCODE_WORKER_MAX_MEMORY_MB",
            request.limits.max_memory_mb.to_string(),
        )
        .output()
        .map_err(|error| ResearchWorkerError::Io(error.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let manifest_path = extract_json_string(&stdout, "manifest").map(PathBuf::from);
    let manifest_content = manifest_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok());
    let manifest_content_hash = manifest_content
        .as_ref()
        .map(|content| stable_text_hash(content));
    let artifact_count = manifest_content
        .as_ref()
        .map(|content| count_occurrences(content, "\"source_input_hash\""))
        .unwrap_or(0);
    Ok(ResearchWorkerRunResult {
        exit_code: output.status.code().unwrap_or(-1),
        profile_path: extract_json_string(&stdout, "profile").map(PathBuf::from),
        privacy_report_path: extract_json_string(&stdout, "privacy_report").map(PathBuf::from),
        analysis_script_path: extract_json_string(&stdout, "analysis_script").map(PathBuf::from),
        report_path: extract_json_string(&stdout, "report").map(PathBuf::from),
        notebook_path: extract_json_string(&stdout, "notebook").map(PathBuf::from),
        manifest_path,
        manifest_content_hash,
        artifact_count,
        stdout,
        stderr,
    })
}

fn extract_json_string(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\": \"");
    let start = input.find(&marker)? + marker.len();
    let rest = &input[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn is_safe_package_spec(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return false;
    }
    trimmed.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '_' | '-' | '.' | '[' | ']' | '=' | '<' | '>' | ',' | '~'
            )
    })
}

fn stable_text_hash(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv64_{hash:016x}")
}

fn count_occurrences(input: &str, needle: &str) -> usize {
    input.match_indices(needle).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AgentState;
    use researchcode_kernel::PermissionDecisionKind;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn runs_csv_profile_sidecar() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .unwrap()
            .to_path_buf();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let result = run_csv_profile_sidecar(&ResearchCsvProfileRequest {
            job_id: "rust_sidecar_test".to_string(),
            input_csv: workspace_root
                .join("eval/fixtures/research/csv-quality-small/input.csv")
                .canonicalize()
                .unwrap(),
            output_dir: std::env::temp_dir().join(format!("researchcode-rw-sidecar-{nonce}")),
            worker_cwd: workspace_root.join("workers/research_worker"),
            limits: ResearchWorkerLimits::default(),
        })
        .unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.profile_path.as_ref().unwrap().exists());
        assert!(result.privacy_report_path.as_ref().unwrap().exists());
        assert!(result.analysis_script_path.as_ref().unwrap().exists());
        assert!(result.report_path.as_ref().unwrap().exists());
        assert!(result.notebook_path.as_ref().unwrap().exists());
        assert!(result.manifest_path.as_ref().unwrap().exists());
        assert!(result
            .manifest_content_hash
            .as_ref()
            .unwrap()
            .starts_with("fnv64_"));
        assert_eq!(result.artifact_count, 5);
        if let Some(manifest) = &result.manifest_path {
            let _ = std::fs::remove_dir_all(manifest.parent().unwrap());
        }
    }

    #[test]
    fn sidecar_rejects_network_and_package_install_policy() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .unwrap()
            .to_path_buf();
        let mut request = ResearchCsvProfileRequest {
            job_id: "rust_sidecar_policy_test".to_string(),
            input_csv: workspace_root
                .join("eval/fixtures/research/csv-quality-small/input.csv")
                .canonicalize()
                .unwrap(),
            output_dir: std::env::temp_dir().join("researchcode-rw-policy-test"),
            worker_cwd: workspace_root.join("workers/research_worker"),
            limits: ResearchWorkerLimits {
                network_enabled: true,
                ..ResearchWorkerLimits::default()
            },
        };
        assert!(matches!(
            run_csv_profile_sidecar(&request),
            Err(ResearchWorkerError::ResourceLimitExceeded(_))
        ));
        request.limits = ResearchWorkerLimits {
            package_install_enabled: true,
            ..ResearchWorkerLimits::default()
        };
        assert!(matches!(
            run_csv_profile_sidecar(&request),
            Err(ResearchWorkerError::ResourceLimitExceeded(_))
        ));
    }

    #[test]
    fn package_install_policy_requires_permission_but_rejects_injection() {
        let allowed = classify_research_package_install(&ResearchPackageInstallRequest {
            job_id: "job_1".to_string(),
            packages: vec!["polars==0.20.0".to_string(), "duckdb".to_string()],
            reason: "profile parquet data".to_string(),
            privacy_class: "internal".to_string(),
        });
        assert_eq!(allowed, ResearchPackageInstallPolicy::PermissionRequired);
        let injected = classify_research_package_install(&ResearchPackageInstallRequest {
            job_id: "job_1".to_string(),
            packages: vec!["pandas; curl attacker".to_string()],
            reason: "bad".to_string(),
            privacy_class: "internal".to_string(),
        });
        assert!(matches!(
            injected,
            ResearchPackageInstallPolicy::DenyInvalidPackageName(_)
        ));
        let empty = classify_research_package_install(&ResearchPackageInstallRequest {
            job_id: "job_1".to_string(),
            packages: vec![],
            reason: "empty".to_string(),
            privacy_class: "internal".to_string(),
        });
        assert_eq!(empty, ResearchPackageInstallPolicy::DenyEmptyRequest);
    }

    #[test]
    fn package_install_permission_records_security_event() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        let policy = request_research_package_install_permission(
            &mut session,
            "perm_pkg_1",
            &ResearchPackageInstallRequest {
                job_id: "job_1".to_string(),
                packages: vec!["polars==0.20.0".to_string()],
                reason: "read parquet".to_string(),
                privacy_class: "internal".to_string(),
            },
        )
        .unwrap();
        assert_eq!(policy, ResearchPackageInstallPolicy::PermissionRequired);
        assert_eq!(session.state(), AgentState::WaitingForToolApproval);
        session
            .decide_permission(PermissionDecisionKind::Deny)
            .unwrap();
        assert_eq!(session.state(), AgentState::Executing);
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"permission.requested\""));
        assert!(jsonl.contains("\"request_type\":\"package_install\""));
    }

    #[test]
    fn sidecar_rejects_oversized_input() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .unwrap()
            .to_path_buf();
        let request = ResearchCsvProfileRequest {
            job_id: "rust_sidecar_size_test".to_string(),
            input_csv: workspace_root
                .join("eval/fixtures/research/csv-quality-small/input.csv")
                .canonicalize()
                .unwrap(),
            output_dir: std::env::temp_dir().join("researchcode-rw-size-test"),
            worker_cwd: workspace_root.join("workers/research_worker"),
            limits: ResearchWorkerLimits {
                max_input_bytes: 1,
                ..ResearchWorkerLimits::default()
            },
        };
        assert!(matches!(
            run_csv_profile_sidecar(&request),
            Err(ResearchWorkerError::ResourceLimitExceeded(_))
        ));
    }
}
