//! Tool manifest and provider schema generation for native TCML.

use super::contract::resolve_tool_name;
use researchcode_kernel::model::NativeModelFamily;
use researchcode_kernel::tool::{
    core_tool_specs, find_tool_spec, provider_tool_name_for_id, provider_tool_name_for_spec,
    tool_catalog_hash, ToolCapabilityStatus, ToolSpec,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolManifest {
    pub manifest_hash: String,
    pub provider_tool_names: Vec<String>,
    pub canonical_tool_ids: Vec<String>,
    pub model_family: String,
    pub provider_protocol: String,
    pub tool_exposure: String,
    pub workflow_state: String,
    pub visible_tool_count: usize,
    pub hidden_tool_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolManifestExposure {
    ReadOnly,
    FastAutoWrite,
    CodeEdit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolManifestBuildContext {
    pub family: NativeModelFamily,
    pub protocol: String,
    pub exposure: ToolManifestExposure,
    pub workflow_state: String,
    pub permission_summary: String,
    pub task_contract_mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltToolManifest {
    pub manifest: ToolManifest,
    pub tool_schema_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDoctorReport {
    pub ok: bool,
    pub manifest_hash: String,
    pub checked_tools: usize,
    pub failures: Vec<String>,
}

pub fn build_tool_manifest() -> ToolManifest {
    build_tool_manifest_for_context(&ToolManifestBuildContext {
        family: NativeModelFamily::DeepSeek,
        protocol: "openai_compatible".to_string(),
        exposure: ToolManifestExposure::ReadOnly,
        workflow_state: "executing".to_string(),
        permission_summary: "default".to_string(),
        task_contract_mode: "default".to_string(),
    })
    .manifest
}

pub fn build_tool_manifest_for_context(context: &ToolManifestBuildContext) -> BuiltToolManifest {
    let all_specs = core_tool_specs();
    let visible_specs = all_specs
        .iter()
        .filter(|tool| allow_tool_for_manifest(tool, &context.exposure))
        .cloned()
        .collect::<Vec<_>>();
    let manifest = ToolManifest {
        manifest_hash: tool_catalog_hash(),
        provider_tool_names: visible_specs
            .iter()
            .map(provider_tool_name_for_spec)
            .collect(),
        canonical_tool_ids: visible_specs
            .iter()
            .map(|tool| tool.tool_id.clone())
            .collect(),
        model_family: match context.family {
            NativeModelFamily::DeepSeek => "deepseek".to_string(),
            NativeModelFamily::Qwen => "qwen".to_string(),
        },
        provider_protocol: context.protocol.clone(),
        tool_exposure: manifest_exposure_label(&context.exposure).to_string(),
        workflow_state: context.workflow_state.clone(),
        visible_tool_count: visible_specs.len(),
        hidden_tool_count: all_specs.len().saturating_sub(visible_specs.len()),
    };
    BuiltToolManifest {
        tool_schema_json: tool_schema_json_for_protocol(&visible_specs, &context.protocol),
        manifest,
    }
}

pub fn tool_manifest_generated_payload_json(manifest: &ToolManifest) -> String {
    format!(
        "{{\"manifest_hash\":{},\"model_family\":{},\"protocol\":{},\"tool_exposure\":{},\"workflow_state\":{},\"visible_tool_count\":{},\"hidden_tool_count\":{},\"provider_tool_names\":{},\"canonical_tool_ids\":{}}}",
        json_string(&manifest.manifest_hash),
        json_string(&manifest.model_family),
        json_string(&manifest.provider_protocol),
        json_string(&manifest.tool_exposure),
        json_string(&manifest.workflow_state),
        manifest.visible_tool_count,
        manifest.hidden_tool_count,
        json_array_strings(&manifest.provider_tool_names),
        json_array_strings(&manifest.canonical_tool_ids)
    )
}

pub fn run_tool_manifest_doctor() -> ToolDoctorReport {
    let specs = core_tool_specs();
    let mut failures = Vec::new();
    for spec in specs {
        if find_tool_spec(&spec.tool_id).is_none() {
            failures.push(format!("registered spec not findable: {}", spec.tool_id));
        }
        if spec.input_schema_json.trim().is_empty() {
            failures.push(format!("missing input schema: {}", spec.tool_id));
        }
        let provider_name = provider_tool_name_for_id(&spec.tool_id);
        let normalized = resolve_tool_name(&provider_name).canonical_tool_id;
        if normalized != spec.tool_id {
            failures.push(format!(
                "provider alias does not resolve: {} -> {} expected {}",
                provider_name, normalized, spec.tool_id
            ));
        }
    }
    ToolDoctorReport {
        ok: failures.is_empty(),
        manifest_hash: build_tool_manifest().manifest_hash,
        checked_tools: specs.len(),
        failures,
    }
}

fn allow_tool_for_manifest(tool: &ToolSpec, _exposure: &ToolManifestExposure) -> bool {
    tool.enabled_by_default && !matches!(tool.capability_status, ToolCapabilityStatus::Gated)
}

fn manifest_exposure_label(exposure: &ToolManifestExposure) -> &'static str {
    match exposure {
        ToolManifestExposure::ReadOnly => "read_only",
        ToolManifestExposure::FastAutoWrite => "fast_auto_write",
        ToolManifestExposure::CodeEdit => "code_edit",
    }
}

fn tool_schema_json_for_protocol(specs: &[ToolSpec], protocol: &str) -> String {
    if protocol == "anthropic_compatible" {
        let entries = specs
            .iter()
            .map(|tool| {
                format!(
                    "{{\"name\":{},\"description\":{},\"input_schema\":{}}}",
                    json_string(&provider_tool_name_for_spec(tool)),
                    json_string(&tool.description),
                    tool.input_schema_json
                )
            })
            .collect::<Vec<_>>();
        return format!("[{}]", entries.join(","));
    }
    let entries = specs
        .iter()
        .map(|tool| {
            format!(
                "{{\"type\":\"function\",\"function\":{{\"name\":{},\"description\":{},\"parameters\":{}}}}}",
                json_string(&provider_tool_name_for_spec(tool)),
                json_string(&tool.description),
                tool.input_schema_json
            )
        })
        .collect::<Vec<_>>();
    format!("[{}]", entries.join(","))
}

fn json_array_strings(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| json_string(value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn json_string(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other if other.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", other as u32));
            }
            other => escaped.push(other),
        }
    }
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_doctor_passes() {
        let report = run_tool_manifest_doctor();
        assert!(report.ok, "{:?}", report.failures);
        assert!(!report.manifest_hash.is_empty());
    }

    #[test]
    fn context_manifest_keeps_permission_gated_tools_visible_in_read_only_mode() {
        let built = build_tool_manifest_for_context(&ToolManifestBuildContext {
            family: NativeModelFamily::DeepSeek,
            protocol: "anthropic_compatible".to_string(),
            exposure: ToolManifestExposure::ReadOnly,
            workflow_state: "executing".to_string(),
            permission_summary: "default".to_string(),
            task_contract_mode: "default".to_string(),
        });
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.read".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.write".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"shell.command".to_string()));
        assert!(built.tool_schema_json.contains("\"name\":\"file_read\""));
        assert!(built.tool_schema_json.contains("\"name\":\"file_write\""));
        assert!(built
            .tool_schema_json
            .contains("\"name\":\"shell_command\""));
    }

    #[test]
    fn context_manifest_code_edit_keeps_read_before_edit_surface() {
        let built = build_tool_manifest_for_context(&ToolManifestBuildContext {
            family: NativeModelFamily::DeepSeek,
            protocol: "anthropic_compatible".to_string(),
            exposure: ToolManifestExposure::CodeEdit,
            workflow_state: "editing".to_string(),
            permission_summary: "default".to_string(),
            task_contract_mode: "default".to_string(),
        });
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.read".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.edit".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.write".to_string()));
        assert!(built.tool_schema_json.contains("\"name\":\"file_read\""));
        assert!(built.tool_schema_json.contains("\"name\":\"file_edit\""));
    }

    #[test]
    fn context_manifest_fast_auto_write_exposes_permission_gated_tools() {
        let built = build_tool_manifest_for_context(&ToolManifestBuildContext {
            family: NativeModelFamily::Qwen,
            protocol: "openai_compatible".to_string(),
            exposure: ToolManifestExposure::FastAutoWrite,
            workflow_state: "executing".to_string(),
            permission_summary: "default".to_string(),
            task_contract_mode: "default".to_string(),
        });
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.write".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.read".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"search.ripgrep".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"shell.command".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"patch.apply".to_string()));
        assert!(built.tool_schema_json.contains("\"type\":\"function\""));
    }

    #[test]
    fn context_manifest_writing_state_does_not_hide_read_or_validation_surface() {
        let built = build_tool_manifest_for_context(&ToolManifestBuildContext {
            family: NativeModelFamily::DeepSeek,
            protocol: "anthropic_compatible".to_string(),
            exposure: ToolManifestExposure::FastAutoWrite,
            workflow_state: "writing".to_string(),
            permission_summary: "default".to_string(),
            task_contract_mode: "default".to_string(),
        });
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.write".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"plan.write".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.read".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.list_directory".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.list_tree".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"search.ripgrep".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"shell.command".to_string()));
        assert!(built.tool_schema_json.contains("\"name\":\"file_write\""));
        assert!(built.tool_schema_json.contains("\"name\":\"file_read\""));
        assert!(built
            .tool_schema_json
            .contains("\"name\":\"list_directory\""));
    }

    #[test]
    fn context_manifest_planning_state_keeps_stable_tool_surface() {
        let built = build_tool_manifest_for_context(&ToolManifestBuildContext {
            family: NativeModelFamily::DeepSeek,
            protocol: "anthropic_compatible".to_string(),
            exposure: ToolManifestExposure::ReadOnly,
            workflow_state: "planning".to_string(),
            permission_summary: "default".to_string(),
            task_contract_mode: "default".to_string(),
        });
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"repo.map".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"search.ripgrep".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.read".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"git.status".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.write".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"shell.command".to_string()));
    }

    #[test]
    fn context_manifest_fast_auto_write_ignores_testing_state_filter() {
        let built = build_tool_manifest_for_context(&ToolManifestBuildContext {
            family: NativeModelFamily::Qwen,
            protocol: "openai_compatible".to_string(),
            exposure: ToolManifestExposure::FastAutoWrite,
            workflow_state: "testing".to_string(),
            permission_summary: "default".to_string(),
            task_contract_mode: "default".to_string(),
        });
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"shell.command".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.read".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"patch.apply".to_string()));
    }

    #[test]
    fn context_manifest_folder_summary_state_keeps_stable_tool_surface() {
        let built = build_tool_manifest_for_context(&ToolManifestBuildContext {
            family: NativeModelFamily::DeepSeek,
            protocol: "openai_compatible".to_string(),
            exposure: ToolManifestExposure::ReadOnly,
            workflow_state: "folder_summary".to_string(),
            permission_summary: "default".to_string(),
            task_contract_mode: "default".to_string(),
        });
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.list_directory".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"file.list_tree".to_string()));
        assert!(built
            .manifest
            .canonical_tool_ids
            .contains(&"shell.command".to_string()));
    }

    #[test]
    fn manifest_tool_set_is_stable_across_exposure_and_workflow_state() {
        let read_only = build_tool_manifest_for_context(&ToolManifestBuildContext {
            family: NativeModelFamily::DeepSeek,
            protocol: "openai_compatible".to_string(),
            exposure: ToolManifestExposure::ReadOnly,
            workflow_state: "planning".to_string(),
            permission_summary: "default".to_string(),
            task_contract_mode: "default".to_string(),
        });
        let code_edit = build_tool_manifest_for_context(&ToolManifestBuildContext {
            family: NativeModelFamily::DeepSeek,
            protocol: "openai_compatible".to_string(),
            exposure: ToolManifestExposure::CodeEdit,
            workflow_state: "editing".to_string(),
            permission_summary: "default".to_string(),
            task_contract_mode: "default".to_string(),
        });
        assert_eq!(
            read_only.manifest.canonical_tool_ids,
            code_edit.manifest.canonical_tool_ids
        );
        assert_eq!(
            read_only.manifest.provider_tool_names,
            code_edit.manifest.provider_tool_names
        );
        assert_eq!(
            read_only.manifest.hidden_tool_count,
            code_edit.manifest.hidden_tool_count
        );
    }
}
