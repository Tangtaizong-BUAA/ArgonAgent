//! Research Worker harness fixtures.

use crate::research_worker::{
    classify_research_package_install, run_csv_profile_sidecar, ResearchCsvProfileRequest,
    ResearchPackageInstallPolicy, ResearchPackageInstallRequest, ResearchWorkerError,
    ResearchWorkerLimits,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchHarnessCaseResult {
    pub case_id: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchHarnessSuiteResult {
    pub passed: bool,
    pub cases: Vec<ResearchHarnessCaseResult>,
}

impl ResearchHarnessSuiteResult {
    pub fn passed_count(&self) -> usize {
        self.cases.iter().filter(|case| case.passed).count()
    }

    pub fn to_summary_line(&self) -> String {
        format!(
            "research harness passed={}/{} ok={}",
            self.passed_count(),
            self.cases.len(),
            self.passed
        )
    }
}

pub fn run_research_harness_suite() -> ResearchHarnessSuiteResult {
    let mut cases = Vec::new();
    cases.push(case_profile_fixture_manifest_lineage());
    cases.push(case_sensitive_column_requires_cloud_approval());
    cases.push(case_oversized_input_rejected());
    cases.push(case_network_and_package_limits_rejected());
    cases.push(case_package_install_classifier_boundaries());
    let passed = cases.iter().all(|case| case.passed);
    ResearchHarnessSuiteResult { passed, cases }
}

fn case_profile_fixture_manifest_lineage() -> ResearchHarnessCaseResult {
    let workspace_root = workspace_root();
    let output_dir = temp_root("research-harness-lineage");
    let result = run_csv_profile_sidecar(&ResearchCsvProfileRequest {
        job_id: "research_harness_lineage".to_string(),
        input_csv: workspace_root
            .join("eval/fixtures/research/csv-quality-small/input.csv")
            .canonicalize()
            .unwrap(),
        output_dir: output_dir.clone(),
        worker_cwd: workspace_root.join("workers/research_worker"),
        limits: ResearchWorkerLimits::default(),
    });
    let passed = result.as_ref().is_ok_and(|value| {
        value.exit_code == 0
            && value.artifact_count == 5
            && value
                .manifest_content_hash
                .as_deref()
                .unwrap_or("")
                .starts_with("fnv64_")
            && value
                .manifest_path
                .as_ref()
                .and_then(|path| fs::read_to_string(path).ok())
                .is_some_and(|text| {
                    text.contains("\"data_lineage\"")
                        && text.contains("\"source_input_hash\"")
                        && text.contains("\"artifact_count\": 5")
                })
    });
    let detail = format!("{result:?}");
    let _ = fs::remove_dir_all(output_dir);
    case("profile_fixture_manifest_lineage", passed, detail)
}

fn case_sensitive_column_requires_cloud_approval() -> ResearchHarnessCaseResult {
    let workspace_root = workspace_root();
    let root = temp_root("research-harness-sensitive");
    let input = root.join("subjects.csv");
    fs::write(
        &input,
        "subject_email,value\nuser@example.com,1\nother@example.com,2\n,3\n",
    )
    .unwrap();
    let output_dir = root.join("out");
    let result = run_csv_profile_sidecar(&ResearchCsvProfileRequest {
        job_id: "research_harness_sensitive".to_string(),
        input_csv: input,
        output_dir: output_dir.clone(),
        worker_cwd: workspace_root.join("workers/research_worker"),
        limits: ResearchWorkerLimits::default(),
    });
    let passed = result.as_ref().is_ok_and(|value| {
        value
            .privacy_report_path
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok())
            .is_some_and(|text| {
                text.contains("\"cloud_model_requires_approval\": true")
                    && text.contains("\"sensitive_column_count\": 1")
                    && text.contains("subject_email")
            })
    });
    let detail = format!("{result:?}");
    let _ = fs::remove_dir_all(root);
    case("sensitive_column_requires_cloud_approval", passed, detail)
}

fn case_oversized_input_rejected() -> ResearchHarnessCaseResult {
    let workspace_root = workspace_root();
    let root = temp_root("research-harness-oversized");
    let input = root.join("large.csv");
    fs::write(&input, "a\n123456\n").unwrap();
    let result = run_csv_profile_sidecar(&ResearchCsvProfileRequest {
        job_id: "research_harness_oversized".to_string(),
        input_csv: input,
        output_dir: root.join("out"),
        worker_cwd: workspace_root.join("workers/research_worker"),
        limits: ResearchWorkerLimits {
            max_input_bytes: 1,
            ..ResearchWorkerLimits::default()
        },
    });
    let passed = matches!(
        result,
        Err(ResearchWorkerError::ResourceLimitExceeded(ref message))
            if message.contains("max_input_bytes")
    );
    let detail = format!("{result:?}");
    let _ = fs::remove_dir_all(root);
    case("oversized_input_rejected", passed, detail)
}

fn case_network_and_package_limits_rejected() -> ResearchHarnessCaseResult {
    let workspace_root = workspace_root();
    let root = temp_root("research-harness-limits");
    let input = root.join("input.csv");
    fs::write(&input, "a\n1\n").unwrap();
    let network = run_csv_profile_sidecar(&ResearchCsvProfileRequest {
        job_id: "research_harness_network".to_string(),
        input_csv: input.clone(),
        output_dir: root.join("out_network"),
        worker_cwd: workspace_root.join("workers/research_worker"),
        limits: ResearchWorkerLimits {
            network_enabled: true,
            ..ResearchWorkerLimits::default()
        },
    });
    let package = run_csv_profile_sidecar(&ResearchCsvProfileRequest {
        job_id: "research_harness_package".to_string(),
        input_csv: input,
        output_dir: root.join("out_package"),
        worker_cwd: workspace_root.join("workers/research_worker"),
        limits: ResearchWorkerLimits {
            package_install_enabled: true,
            ..ResearchWorkerLimits::default()
        },
    });
    let passed = matches!(network, Err(ResearchWorkerError::ResourceLimitExceeded(_)))
        && matches!(package, Err(ResearchWorkerError::ResourceLimitExceeded(_)));
    let detail = format!("network={network:?} package={package:?}");
    let _ = fs::remove_dir_all(root);
    case("network_and_package_limits_rejected", passed, detail)
}

fn case_package_install_classifier_boundaries() -> ResearchHarnessCaseResult {
    let allowed = classify_research_package_install(&ResearchPackageInstallRequest {
        job_id: "pkg_ok".to_string(),
        packages: vec!["pandas==2.2.0".to_string(), "polars>=0.20".to_string()],
        reason: "profile parquet".to_string(),
        privacy_class: "internal".to_string(),
    });
    let injected = classify_research_package_install(&ResearchPackageInstallRequest {
        job_id: "pkg_bad".to_string(),
        packages: vec!["pandas; curl attacker".to_string()],
        reason: "bad".to_string(),
        privacy_class: "internal".to_string(),
    });
    let empty = classify_research_package_install(&ResearchPackageInstallRequest {
        job_id: "pkg_empty".to_string(),
        packages: vec![],
        reason: "bad".to_string(),
        privacy_class: "internal".to_string(),
    });
    let passed = allowed == ResearchPackageInstallPolicy::PermissionRequired
        && matches!(
            injected,
            ResearchPackageInstallPolicy::DenyInvalidPackageName(_)
        )
        && empty == ResearchPackageInstallPolicy::DenyEmptyRequest;
    case(
        "package_install_classifier_boundaries",
        passed,
        format!("allowed={allowed:?} injected={injected:?} empty={empty:?}"),
    )
}

fn case(case_id: &str, passed: bool, detail: String) -> ResearchHarnessCaseResult {
    ResearchHarnessCaseResult {
        case_id: case_id.to_string(),
        passed,
        detail,
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .unwrap()
        .to_path_buf()
}

fn temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-{label}-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn research_harness_suite_covers_worker_boundaries() {
        let result = run_research_harness_suite();
        assert!(result.passed, "{result:?}");
    }
}
