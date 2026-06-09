//! Tool-call mediation layer for native model output.
//!
//! TCML owns provider/model-facing repair and normalization rules. Older parser
//! and contract modules keep compatibility wrappers, but canonical alias
//! resolution should live here.

pub mod alias_registry;
pub mod content_extractor;
pub mod contract;
pub mod error_factory;
pub mod issue_guided_repairer;
pub mod manifest;
pub mod parser;
pub mod pipeline;
pub mod relational_resolver;
pub mod repair_catalog;
pub mod schema_validator;

pub use crate::native_profile::deepseek::stream::{
    CompletedStreamingToolCall, StreamingToolCallAssembler,
};
pub use alias_registry::{canonical_tool_id, normalize_alias_key, AliasRegistry, AliasResolution};
pub use content_extractor::{
    extract_content_tool_call_candidates, scan_content_tool_call_candidates,
    ContentToolCallCandidate,
};
pub use contract::{
    mediate_tool_call, mediate_tool_call_with_provider_id, model_error_to_tool_result,
    MediatedToolCall, ModelReadableToolError, StreamingToolCallAccumulator, ToolCallLedger,
    ToolInputRepair, ToolMediationEvent, ToolMediationStatus,
};
pub use error_factory::{model_readable_tool_error, ToolErrorCode};
pub use issue_guided_repairer::{apply_low_risk_repairs, IssueGuidedRepairer};
pub use manifest::{
    build_tool_manifest, build_tool_manifest_for_context, run_tool_manifest_doctor,
    tool_manifest_generated_payload_json, BuiltToolManifest, ToolDoctorReport, ToolManifest,
    ToolManifestBuildContext, ToolManifestExposure,
};
pub use parser::{
    extract_json_bool, extract_json_string, extract_json_value, normalize_tool_id,
    parse_first_tool_call, parse_tool_arguments, parse_tool_calls,
    strip_tool_call_markup_from_visible_text, visible_text_without_tool_calls, ParsedToolArguments,
    ParsedToolCall, ToolCallParseStatus, ToolCallSyntax,
};
pub use pipeline::{PipelineOutcome, ToolCallPipeline};
pub use relational_resolver::{file_read_relational_default, FileReadRelationalDefault};
pub use repair_catalog::{
    can_repair_field, markdown_link_path_target, optional_null_present, quoted_usize_argument,
    OPTIONAL_NULL_REPAIR_KEYS,
};
pub use schema_validator::{
    is_required_key, required_keys_for_tool, validate_required_arguments, SchemaValidator,
};
