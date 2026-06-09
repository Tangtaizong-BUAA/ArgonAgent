//! Product Kernel ToolSpec primitives.

use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCategory {
    File,
    Search,
    Shell,
    Git,
    Patch,
    Plan,
    Todo,
    Question,
    Lsp,
    Research,
    Artifact,
    Worktree,
    Notebook,
    Web,
    Browser,
    Mcp,
    Agent,
    Skill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRisk {
    ReadOnly,
    WritesFiles,
    ExecutesCommand,
    UsesNetwork,
    ExportsArtifact,
    Interactive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolResultPolicy {
    Inline,
    PreviewAndArtifact,
    ArtifactOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPermissionKind {
    None,
    Command,
    FileWrite,
    Network,
    PackageInstall,
    CloudModel,
    ProtectedPath,
    ArtifactExport,
    PlanApproval,
    UserQuestion,
    ExternalPlugin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRenderer {
    Text,
    ToolCallCard,
    PermissionCard,
    DiffCard,
    CommandResultCard,
    TodoPanel,
    PlanCard,
    ResearchArtifactCard,
    GatedCapabilityCard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolTruncationPolicy {
    None,
    Tail,
    MiddleSummary,
    ArtifactReference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolModelCompatibility {
    All,
    NativeOnly,
    DeepSeekNative,
    QwenNative,
    CompatibleProvider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCapabilityStatus {
    Production,
    GovernanceOnly,
    PreviewOnly,
    Gated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpec {
    pub tool_id: String,
    pub display_name: String,
    pub category: ToolCategory,
    pub risk: ToolRisk,
    pub description: String,
    pub provider_aliases: Vec<String>,
    pub input_schema_json: String,
    pub output_schema_json: String,
    pub permission_kind: ToolPermissionKind,
    pub path_argument_keys: Vec<String>,
    pub renderer: ToolRenderer,
    pub truncation_policy: ToolTruncationPolicy,
    pub model_compatibility: ToolModelCompatibility,
    pub capability_status: ToolCapabilityStatus,
    pub permission_required: bool,
    pub enabled_by_default: bool,
    pub concurrency_safe: bool,
    pub max_result_size_chars: usize,
    pub result_policy: ToolResultPolicy,
}

static GATED_TOOL_SPECS: LazyLock<Vec<ToolSpec>> = LazyLock::new(|| {
    [
        ("worktree.create", "Create Worktree", ToolCategory::Worktree, ToolRisk::WritesFiles),
        ("worktree.rollback", "Rollback Worktree", ToolCategory::Worktree, ToolRisk::WritesFiles),
        ("notebook.edit", "Edit Notebook", ToolCategory::Notebook, ToolRisk::WritesFiles),
        ("web.fetch", "Fetch Web Page", ToolCategory::Web, ToolRisk::UsesNetwork),
        ("web.search", "Search Web", ToolCategory::Web, ToolRisk::UsesNetwork),
        ("browser.open", "Open Browser", ToolCategory::Browser, ToolRisk::UsesNetwork),
        ("mcp.tool", "MCP Tool", ToolCategory::Mcp, ToolRisk::Interactive),
        ("mcp.resource", "MCP Resource", ToolCategory::Mcp, ToolRisk::ReadOnly),
        ("agent.explorer", "Explorer Subagent", ToolCategory::Agent, ToolRisk::ReadOnly),
        ("agent.reviewer", "Reviewer Subagent", ToolCategory::Agent, ToolRisk::ReadOnly),
        ("agent.worker", "Implementation Subagent", ToolCategory::Agent, ToolRisk::WritesFiles),
        ("skill.run", "Run Skill", ToolCategory::Skill, ToolRisk::Interactive),
    ]
    .into_iter()
    .map(|(tool_id, display_name, category, risk)| {
        let concurrency_safe = matches!(risk, ToolRisk::ReadOnly);
        let mut tool = spec(
            tool_id,
            display_name,
            category,
            risk,
            "Gated ClaudeCode/OpenCode parity capability. Disabled until security and eval gates pass.",
            &[],
            r#"{"type":"object","properties":{}}"#,
            r#"{"type":"object","properties":{"status":{"type":"string"}}}"#,
            ToolPermissionKind::ExternalPlugin,
            &[],
            ToolRenderer::GatedCapabilityCard,
            ToolTruncationPolicy::ArtifactReference,
            ToolResultPolicy::PreviewAndArtifact,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Gated,
            concurrency_safe,
            20_000,
        );
        tool.enabled_by_default = false;
        tool
    })
    .collect()
});

static CORE_TOOL_SPECS: LazyLock<Vec<ToolSpec>> = LazyLock::new(|| {
    let mut tools = vec![
        spec(
            "file.read",
            "Read File",
            ToolCategory::File,
            ToolRisk::ReadOnly,
            "Read a UTF-8 text file inside the workspace with offset/limit metadata.",
            &[
                "file_read",
                "read_file",
                "readFile",
                "read_source_code",
                "read_source",
                "readSource",
                "open_file",
                "view_file",
            ],
            r#"{"type":"object","required":["path"],"properties":{"path":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"},"max_bytes":{"type":"integer"}}}"#,
            r#"{"type":"object","properties":{"content":{"type":"string"},"line_start":{"type":"integer"},"line_end":{"type":"integer"},"content_hash":{"type":"string"},"truncated":{"type":"boolean"}}}"#,
            ToolPermissionKind::None,
            &["path"],
            ToolRenderer::ToolCallCard,
            ToolTruncationPolicy::MiddleSummary,
            ToolResultPolicy::Inline,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Production,
            true,
            80_000,
        ),
        spec(
            "file.list_directory",
            "List Directory",
            ToolCategory::File,
            ToolRisk::ReadOnly,
            "List direct children of a directory inside the workspace.",
            &[
                "list_directory",
                "list_dir",
                "read_directory",
                "directory_ls",
                "dir_ls",
                "file_ls",
                "ls_dir",
            ],
            r#"{"type":"object","required":["path"],"properties":{"path":{"type":"string"},"include_hidden":{"type":"boolean"},"max_entries":{"type":"integer"}}}"#,
            r#"{"type":"object","properties":{"path":{"type":"string"},"entry_count":{"type":"integer"},"entries":{"type":"array"}}}"#,
            ToolPermissionKind::None,
            &["path"],
            ToolRenderer::ToolCallCard,
            ToolTruncationPolicy::MiddleSummary,
            ToolResultPolicy::Inline,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Production,
            true,
            60_000,
        ),
        spec(
            "file.list_tree",
            "List Directory Tree",
            ToolCategory::File,
            ToolRisk::ReadOnly,
            "List a bounded directory tree for quick project structure inspection.",
            &[
                "list_directory_tree",
                "directory_tree",
                "repo_file_tree",
                "read_file_tree",
                "file_tree",
                "project_tree",
                "tree",
            ],
            r#"{"type":"object","required":["path"],"properties":{"path":{"type":"string"},"depth":{"type":"integer"},"max_entries":{"type":"integer"}}}"#,
            r#"{"type":"object","properties":{"path":{"type":"string"},"line_count":{"type":"integer"},"tree_lines":{"type":"array"}}}"#,
            ToolPermissionKind::None,
            &["path"],
            ToolRenderer::ToolCallCard,
            ToolTruncationPolicy::MiddleSummary,
            ToolResultPolicy::Inline,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Production,
            true,
            60_000,
        ),
        spec(
            "file.edit",
            "Edit File",
            ToolCategory::File,
            ToolRisk::WritesFiles,
            "Replace one exact unique old_string in a previously read file.",
            &["file_edit", "edit"],
            r#"{"type":"object","required":["path","old_string","new_string"],"properties":{"path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"},"base_hash":{"type":"string"},"replace_all":{"type":"boolean"}}}"#,
            r#"{"type":"object","properties":{"path":{"type":"string"},"validation":{"type":"string"},"rollback_artifact":{"type":"string"},"diff":{"type":"string"}}}"#,
            ToolPermissionKind::FileWrite,
            &["path"],
            ToolRenderer::DiffCard,
            ToolTruncationPolicy::ArtifactReference,
            ToolResultPolicy::PreviewAndArtifact,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Production,
            true,
            40_000,
        ),
        spec(
            "file.write",
            "Write File",
            ToolCategory::File,
            ToolRisk::WritesFiles,
            "Create or replace a workspace file after protected-path permission checks.",
            &["file_write", "write", "writeFile"],
            r#"{"type":"object","required":["path","content"],"properties":{"path":{"type":"string"},"content":{"type":"string"},"base_hash":{"type":"string"}}}"#,
            r#"{"type":"object","properties":{"path":{"type":"string"},"content_hash":{"type":"string"},"rollback_artifact":{"type":"string"}}}"#,
            ToolPermissionKind::FileWrite,
            &["path"],
            ToolRenderer::DiffCard,
            ToolTruncationPolicy::ArtifactReference,
            ToolResultPolicy::PreviewAndArtifact,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Production,
            true,
            40_000,
        ),
        spec(
            "file.multi_edit",
            "Multi Edit File",
            ToolCategory::File,
            ToolRisk::WritesFiles,
            "Apply multiple ordered exact replacements to one file as one atomic proposal.",
            &["file_multi_edit", "multi_edit"],
            r#"{"type":"object","required":["path","edits"],"properties":{"path":{"type":"string"},"base_hash":{"type":"string"},"edits":{"type":"array","items":{"type":"object","required":["old_string","new_string"],"properties":{"old_string":{"type":"string"},"new_string":{"type":"string"},"replace_all":{"type":"boolean"}}}}}}"#,
            r#"{"type":"object","properties":{"path":{"type":"string"},"edit_count":{"type":"integer"},"rollback_artifact":{"type":"string"},"diff":{"type":"string"}}}"#,
            ToolPermissionKind::FileWrite,
            &["path"],
            ToolRenderer::DiffCard,
            ToolTruncationPolicy::ArtifactReference,
            ToolResultPolicy::PreviewAndArtifact,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Production,
            true,
            40_000,
        ),
        spec(
            "search.ripgrep",
            "Search Files",
            ToolCategory::Search,
            ToolRisk::ReadOnly,
            "Search workspace text in one file or directory with ripgrep-like bounded output.",
            &[
                "search_ripgrep",
                "grep",
                "rg",
                "search_source_code",
                "file.search",
                "file.grep",
            ],
            r#"{"type":"object","required":["pattern"],"properties":{"path":{"type":"string","description":"Workspace file or directory to search. Preferred over root."},"root":{"type":"string","description":"Deprecated compatibility alias for path."},"pattern":{"type":"string"},"max_results":{"type":"integer"}}}"#,
            r#"{"type":"object","properties":{"path":{"type":"string"},"pattern":{"type":"string"},"match_count":{"type":"integer"},"matches":{"type":"array"},"truncated":{"type":"boolean"},"searched_files":{"type":"integer"}}}"#,
            ToolPermissionKind::None,
            &["path", "root"],
            ToolRenderer::ToolCallCard,
            ToolTruncationPolicy::MiddleSummary,
            ToolResultPolicy::PreviewAndArtifact,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Production,
            true,
            40_000,
        ),
        spec(
            "repo.map",
            "Build Repo Map",
            ToolCategory::Search,
            ToolRisk::ReadOnly,
            "Build a bounded project map for context retrieval.",
            &[
                "repo_map",
                "list_source_files",
                "list_files",
                "repo_ls",
                "repo_list_files",
            ],
            r#"{"type":"object","properties":{"root":{"type":"string"},"max_files":{"type":"integer"},"max_depth":{"type":"integer"}}}"#,
            r#"{"type":"object","properties":{"file_count":{"type":"integer"},"omitted_count":{"type":"integer"},"tech_stack":{"type":"array"}}}"#,
            ToolPermissionKind::None,
            &["root"],
            ToolRenderer::ToolCallCard,
            ToolTruncationPolicy::MiddleSummary,
            ToolResultPolicy::PreviewAndArtifact,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Production,
            true,
            40_000,
        ),
        spec(
            "shell.command",
            "Run Shell Command",
            ToolCategory::Shell,
            ToolRisk::ExecutesCommand,
            "Run a classified command without shell expansion unless explicitly approved.",
            &[
                "shell_command",
                "bash",
                "run",
                "shellCommand",
                "shell.run",
                "execute_command",
                "exec_command",
                "exec",
                "run_shell",
                "shell",
            ],
            r#"{"type":"object","required":["command"],"properties":{"command":{"type":"string"},"root":{"type":"string"},"timeout_ms":{"type":"integer"}}}"#,
            r#"{"type":"object","properties":{"exit_code":{"type":"integer"},"stdout_artifact":{"type":"string"},"stderr_artifact":{"type":"string"},"tail":{"type":"string"}}}"#,
            ToolPermissionKind::Command,
            &["root"],
            ToolRenderer::CommandResultCard,
            ToolTruncationPolicy::ArtifactReference,
            ToolResultPolicy::PreviewAndArtifact,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Production,
            false,
            20_000,
        ),
        spec(
            "patch.apply",
            "Apply Patch",
            ToolCategory::Patch,
            ToolRisk::WritesFiles,
            "Apply a structured patch only after fresh base-hash validation.",
            &["patch_apply", "patch.propose"],
            r#"{"type":"object","required":["path","old_string","new_string","base_hash"],"properties":{"path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"},"base_hash":{"type":"string"}}}"#,
            r#"{"type":"object","properties":{"path":{"type":"string"},"validation":{"type":"string"},"rollback_artifact":{"type":"string"}}}"#,
            ToolPermissionKind::FileWrite,
            &["path"],
            ToolRenderer::DiffCard,
            ToolTruncationPolicy::ArtifactReference,
            ToolResultPolicy::PreviewAndArtifact,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Production,
            false,
            40_000,
        ),
        spec(
            "git.status",
            "Git Status",
            ToolCategory::Git,
            ToolRisk::ReadOnly,
            "Read git status for the current workspace without mutation.",
            &["git_status"],
            r#"{"type":"object","properties":{"root":{"type":"string"}}}"#,
            r#"{"type":"object","properties":{"kind":{"type":"string"},"summary":{"type":"string"}}}"#,
            ToolPermissionKind::None,
            &["root"],
            ToolRenderer::ToolCallCard,
            ToolTruncationPolicy::Tail,
            ToolResultPolicy::Inline,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Production,
            true,
            20_000,
        ),
        spec(
            "todo.write",
            "Write Todo List",
            ToolCategory::Todo,
            ToolRisk::Interactive,
            "Update the session todo list for multi-step agent work.",
            &["todo_write"],
            r#"{"type":"object","required":["items"],"properties":{"items":{"type":"array"}}}"#,
            r#"{"type":"object","properties":{"todo_count":{"type":"integer"}}}"#,
            ToolPermissionKind::None,
            &[],
            ToolRenderer::TodoPanel,
            ToolTruncationPolicy::None,
            ToolResultPolicy::Inline,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Production,
            true,
            8_000,
        ),
        spec(
            "plan.enter",
            "Enter Plan Mode",
            ToolCategory::Plan,
            ToolRisk::Interactive,
            "Present a plan and wait for task-governance approval before writes.",
            &["plan_enter", "enter_plan_mode"],
            r#"{"type":"object","required":["plan"],"properties":{"plan":{"type":"string"}}}"#,
            r#"{"type":"object","properties":{"plan_id":{"type":"string"},"status":{"type":"string"}}}"#,
            ToolPermissionKind::PlanApproval,
            &[],
            ToolRenderer::PlanCard,
            ToolTruncationPolicy::None,
            ToolResultPolicy::Inline,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::GovernanceOnly,
            false,
            20_000,
        ),
        spec(
            "plan.exit",
            "Exit Plan Mode",
            ToolCategory::Plan,
            ToolRisk::Interactive,
            "Exit plan mode after plan approval.",
            &["plan_exit", "exit_plan_mode"],
            r#"{"type":"object","properties":{"plan_id":{"type":"string"}}}"#,
            r#"{"type":"object","properties":{"status":{"type":"string"}}}"#,
            ToolPermissionKind::PlanApproval,
            &[],
            ToolRenderer::PlanCard,
            ToolTruncationPolicy::None,
            ToolResultPolicy::Inline,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::GovernanceOnly,
            false,
            4_000,
        ),
        spec(
            "plan.write",
            "Write Plan Artifact",
            ToolCategory::Plan,
            ToolRisk::Interactive,
            "Write the current plan artifact under .researchcode/plans for governance approval.",
            &["plan_write"],
            r#"{"type":"object","required":["content"],"properties":{"content":{"type":"string"},"plan_id":{"type":"string"}}}"#,
            r#"{"type":"object","properties":{"path":{"type":"string"},"content_hash":{"type":"string"}}}"#,
            ToolPermissionKind::None,
            &[],
            ToolRenderer::PlanCard,
            ToolTruncationPolicy::None,
            ToolResultPolicy::Inline,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::GovernanceOnly,
            false,
            20_000,
        ),
        spec(
            "ask_user",
            "Ask User",
            ToolCategory::Question,
            ToolRisk::Interactive,
            "Ask the user a concise clarification question.",
            &["ask_user_question", "question"],
            r#"{"type":"object","required":["question"],"properties":{"question":{"type":"string"}}}"#,
            r#"{"type":"object","properties":{"answer":{"type":"string"},"status":{"type":"string"}}}"#,
            ToolPermissionKind::UserQuestion,
            &[],
            ToolRenderer::PermissionCard,
            ToolTruncationPolicy::None,
            ToolResultPolicy::Inline,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::GovernanceOnly,
            false,
            8_000,
        ),
        spec(
            "lsp.diagnostics",
            "LSP Diagnostics",
            ToolCategory::Lsp,
            ToolRisk::ReadOnly,
            "Read diagnostics if a language server integration is available.",
            &["lsp_diagnostics"],
            r#"{"type":"object","properties":{"path":{"type":"string"}}}"#,
            r#"{"type":"object","properties":{"diagnostics":{"type":"array"},"available":{"type":"boolean"}}}"#,
            ToolPermissionKind::None,
            &["path"],
            ToolRenderer::ToolCallCard,
            ToolTruncationPolicy::MiddleSummary,
            ToolResultPolicy::PreviewAndArtifact,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::PreviewOnly,
            true,
            40_000,
        ),
        spec(
            "research.csv_profile",
            "Profile CSV",
            ToolCategory::Research,
            ToolRisk::ReadOnly,
            "Profile CSV data with artifact manifest and sensitive-column metadata.",
            &["research_csv_profile", "csv_profile"],
            r#"{"type":"object","required":["input_csv"],"properties":{"input_csv":{"type":"string"},"job_id":{"type":"string"},"output_dir":{"type":"string"}}}"#,
            r#"{"type":"object","properties":{"manifest_path":{"type":"string"},"manifest_hash":{"type":"string"},"artifact_count":{"type":"integer"}}}"#,
            ToolPermissionKind::None,
            &["input_csv", "output_dir"],
            ToolRenderer::ResearchArtifactCard,
            ToolTruncationPolicy::ArtifactReference,
            ToolResultPolicy::PreviewAndArtifact,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Production,
            true,
            80_000,
        ),
        spec(
            "artifact.export",
            "Export Artifact",
            ToolCategory::Artifact,
            ToolRisk::ExportsArtifact,
            "Export an artifact outside normal runtime storage after approval.",
            &["artifact_export"],
            r#"{"type":"object","required":["artifact_id","path"],"properties":{"artifact_id":{"type":"string"},"path":{"type":"string"}}}"#,
            r#"{"type":"object","properties":{"path":{"type":"string"},"content_hash":{"type":"string"}}}"#,
            ToolPermissionKind::ArtifactExport,
            &["path"],
            ToolRenderer::PermissionCard,
            ToolTruncationPolicy::ArtifactReference,
            ToolResultPolicy::ArtifactOnly,
            ToolModelCompatibility::All,
            ToolCapabilityStatus::Gated,
            false,
            4_000,
        ),
        spec(
            "task.dispatch",
            "Dispatch Subagent",
            ToolCategory::Agent,
            ToolRisk::Interactive,
            "Start an isolated subagent task with an optional write scope and model role.",
            &["task_dispatch", "dispatch_subagent"],
            r#"{"type":"object","required":["prompt"],"properties":{"prompt":{"type":"string"},"write_scope":{"type":"object","properties":{"paths":{"type":"array","items":{"type":"string"}}}},"model_role":{"type":"string","enum":["compactor","executor","reviewer"]}}}"#,
            r#"{"type":"object","properties":{"task_id":{"type":"string"},"status":{"type":"string"},"summary":{"type":"string"}}}"#,
            ToolPermissionKind::None,
            &["write_scope.paths"],
            ToolRenderer::ToolCallCard,
            ToolTruncationPolicy::MiddleSummary,
            ToolResultPolicy::Inline,
            ToolModelCompatibility::NativeOnly,
            ToolCapabilityStatus::PreviewOnly,
            true,
            12_000,
        ),
    ];
    tools.extend(GATED_TOOL_SPECS.clone());
    tools.sort_by(|left, right| left.tool_id.cmp(&right.tool_id));
    tools
});

pub fn core_tool_specs() -> &'static [ToolSpec] {
    &CORE_TOOL_SPECS
}

pub fn find_tool_spec(tool_id: &str) -> Option<ToolSpec> {
    CORE_TOOL_SPECS
        .iter()
        .find(|spec| spec.tool_id == tool_id)
        .cloned()
}

pub fn tool_catalog_hash() -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for tool in core_tool_specs() {
        for byte in tool.tool_id.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= u64::from(tool.enabled_by_default);
        hash = hash.wrapping_mul(0x100000001b3);
        for byte in tool_capability_status_str(&tool.capability_status).as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        // Include schema and description so that schema changes invalidate cache.
        for byte in tool.input_schema_json.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        for byte in tool.description.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("fnv64_{hash:016x}")
}

pub fn tool_capability_status_str(status: &ToolCapabilityStatus) -> &'static str {
    match status {
        ToolCapabilityStatus::Production => "production",
        ToolCapabilityStatus::GovernanceOnly => "governance_only",
        ToolCapabilityStatus::PreviewOnly => "preview_only",
        ToolCapabilityStatus::Gated => "gated",
    }
}

pub fn provider_tool_name_for_spec(tool: &ToolSpec) -> String {
    tool.provider_aliases
        .first()
        .cloned()
        .unwrap_or_else(|| tool.tool_id.replace('.', "_"))
}

pub fn provider_tool_name_for_id(tool_id: &str) -> String {
    find_tool_spec(tool_id)
        .map(|tool| provider_tool_name_for_spec(&tool))
        .unwrap_or_else(|| tool_id.replace('.', "_"))
}

pub fn native_readonly_provider_tool_schema_json() -> String {
    provider_tool_schema_json(|tool| {
        tool.enabled_by_default
            && !matches!(tool.capability_status, ToolCapabilityStatus::Gated)
            && (matches!(tool.risk, ToolRisk::ReadOnly)
                || matches!(
                    tool.category,
                    ToolCategory::Plan | ToolCategory::Todo | ToolCategory::Question
                ))
    })
}

pub fn native_readonly_openai_tool_schema_json() -> String {
    openai_tool_schema_json(|tool| {
        tool.enabled_by_default
            && !matches!(tool.capability_status, ToolCapabilityStatus::Gated)
            && (matches!(tool.risk, ToolRisk::ReadOnly)
                || matches!(
                    tool.category,
                    ToolCategory::Plan | ToolCategory::Todo | ToolCategory::Question
                ))
    })
}

pub fn tui_fastauto_provider_tool_schema_json() -> String {
    provider_tool_schema_json(|tool| {
        tool.enabled_by_default
            && !matches!(tool.capability_status, ToolCapabilityStatus::Gated)
            && !matches!(tool.tool_id.as_str(), "shell.command" | "patch.apply")
    })
}

pub fn tui_fastauto_openai_tool_schema_json() -> String {
    openai_tool_schema_json(|tool| {
        tool.enabled_by_default
            && !matches!(tool.capability_status, ToolCapabilityStatus::Gated)
            && !matches!(tool.tool_id.as_str(), "shell.command" | "patch.apply")
    })
}

fn provider_tool_schema_json(allow: impl Fn(&ToolSpec) -> bool) -> String {
    let entries = CORE_TOOL_SPECS
        .iter()
        .filter(|tool| allow(tool))
        .map(|tool| {
            format!(
                "{{\"name\":\"{}\",\"description\":\"{}\",\"input_schema\":{}}}",
                json_escape(&provider_tool_name_for_spec(tool)),
                json_escape(&provider_tool_description(tool)),
                tool.input_schema_json
            )
        })
        .collect::<Vec<_>>();
    format!("[{}]", entries.join(","))
}

fn openai_tool_schema_json(allow: impl Fn(&ToolSpec) -> bool) -> String {
    let entries = CORE_TOOL_SPECS
        .iter()
        .filter(|tool| allow(tool))
        .map(|tool| {
            format!(
                "{{\"type\":\"function\",\"function\":{{\"name\":\"{}\",\"description\":\"{}\",\"parameters\":{}}}}}",
                json_escape(&provider_tool_name_for_spec(tool)),
                json_escape(&provider_tool_description(tool)),
                tool.input_schema_json
            )
        })
        .collect::<Vec<_>>();
    format!("[{}]", entries.join(","))
}

fn provider_tool_description(tool: &ToolSpec) -> String {
    let base = match tool.tool_id.as_str() {
        "file.read" => {
            "Read one concrete UTF-8 text file. Do not use this on directories; use repo_map first for directories."
        }
        "file.write" => {
            "Create or write a UTF-8 text file inside the workspace. Runtime applies write safeguards and preserves the complete content argument."
        }
        "file.edit" => {
            "Replace one exact old_string in a previously read file. Runtime checks stale reads and unique matches."
        }
        "file.multi_edit" => {
            "Apply multiple ordered exact replacements to one file as one atomic validated edit."
        }
        "repo.map" => "Map a directory or repository before reading concrete files.",
        "search.ripgrep" => {
            "Search one workspace file or directory with ripgrep-style bounded output. Prefer path for the target; root is accepted as a compatibility alias."
        }
        "ask_user" => {
            "Ask the user one concise clarification question only when required. Avoid this when a reasonable default satisfies the task."
        }
        "plan.enter" => {
            "Enter plan mode and request task-governance approval before implementation. This is not a safety PermissionRequest."
        }
        "plan.write" => {
            "Write or update the draft plan artifact under .researchcode/plans for governance approval."
        }
        _ => tool.description.as_str(),
    };
    format!(
        "{base} Internal tool id: {}. Capability status: {}.",
        tool.tool_id,
        tool_capability_status_str(&tool.capability_status)
    )
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            other if other.is_control() => format!("\\u{:04x}", other as u32).chars().collect(),
            other => vec![other],
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn spec(
    tool_id: &str,
    display_name: &str,
    category: ToolCategory,
    risk: ToolRisk,
    description: &str,
    provider_aliases: &[&str],
    input_schema_json: &str,
    output_schema_json: &str,
    permission_kind: ToolPermissionKind,
    path_argument_keys: &[&str],
    renderer: ToolRenderer,
    truncation_policy: ToolTruncationPolicy,
    result_policy: ToolResultPolicy,
    model_compatibility: ToolModelCompatibility,
    capability_status: ToolCapabilityStatus,
    concurrency_safe: bool,
    max_result_size_chars: usize,
) -> ToolSpec {
    let permission_required = !matches!(
        permission_kind,
        ToolPermissionKind::None | ToolPermissionKind::UserQuestion
    );
    let enabled_by_default = !matches!(capability_status, ToolCapabilityStatus::Gated);
    ToolSpec {
        tool_id: tool_id.to_string(),
        display_name: display_name.to_string(),
        category,
        risk,
        description: description.to_string(),
        provider_aliases: provider_aliases
            .iter()
            .map(|value| value.to_string())
            .collect(),
        input_schema_json: input_schema_json.to_string(),
        output_schema_json: output_schema_json.to_string(),
        permission_kind,
        path_argument_keys: path_argument_keys
            .iter()
            .map(|value| value.to_string())
            .collect(),
        renderer,
        truncation_policy,
        model_compatibility,
        capability_status,
        permission_required,
        enabled_by_default,
        concurrency_safe,
        max_result_size_chars,
        result_policy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn core_tools_have_unique_ids() {
        let tools = core_tool_specs();
        let unique = tools
            .iter()
            .map(|tool| tool.tool_id.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(unique.len(), tools.len());
    }

    #[test]
    fn write_and_shell_tools_require_permission() {
        assert!(find_tool_spec("shell.command").unwrap().permission_required);
        assert!(find_tool_spec("patch.apply").unwrap().permission_required);
        assert!(!find_tool_spec("file.read").unwrap().permission_required);
    }

    #[test]
    fn capability_status_matches_enabled_boundary() {
        assert_eq!(
            find_tool_spec("file.read").unwrap().capability_status,
            ToolCapabilityStatus::Production
        );
        assert_eq!(
            find_tool_spec("plan.enter").unwrap().capability_status,
            ToolCapabilityStatus::GovernanceOnly
        );
        assert_eq!(
            find_tool_spec("lsp.diagnostics").unwrap().capability_status,
            ToolCapabilityStatus::PreviewOnly
        );
        let artifact_export = find_tool_spec("artifact.export").unwrap();
        assert_eq!(
            artifact_export.capability_status,
            ToolCapabilityStatus::Gated
        );
        assert!(!artifact_export.enabled_by_default);
    }

    #[test]
    fn provider_schema_excludes_gated_tools() {
        let schema = tui_fastauto_provider_tool_schema_json();
        assert!(schema.contains("\"name\":\"file_read\""));
        assert!(schema.contains("\"name\":\"file_write\""));
        assert!(schema.contains("\"name\":\"plan_enter\""));
        assert!(!schema.contains("artifact_export"));
        assert!(!schema.contains("worktree_create"));
        assert!(!schema.contains("shell_command"));
        assert!(!schema.contains("patch_apply"));
    }

    #[test]
    fn readonly_provider_schema_excludes_write_tools() {
        let schema = native_readonly_provider_tool_schema_json();
        assert!(schema.contains("\"name\":\"file_read\""));
        assert!(schema.contains("\"name\":\"repo_map\""));
        assert!(schema.contains("\"name\":\"plan_enter\""));
        assert!(!schema.contains("\"name\":\"file_write\""));
        assert!(!schema.contains("\"name\":\"patch_apply\""));
    }

    #[test]
    fn search_schema_exposes_file_or_directory_path_target() {
        let spec = find_tool_spec("search.ripgrep").unwrap();
        assert!(spec.description.contains("file or directory"));
        assert_eq!(spec.path_argument_keys, vec!["path", "root"]);
        assert!(spec.input_schema_json.contains("\"path\""));
        assert!(spec.input_schema_json.contains("Preferred over root"));
        let schema = native_readonly_provider_tool_schema_json();
        assert!(schema.contains("Prefer path for the target"));
        assert!(schema.contains("Deprecated compatibility alias"));
    }

    #[test]
    fn openai_tool_schema_uses_function_wrappers_for_qwen() {
        let schema = tui_fastauto_openai_tool_schema_json();
        assert!(schema.contains("\"type\":\"function\""));
        assert!(schema.contains("\"function\":{\"name\":\"file_write\""));
        assert!(schema.contains("\"parameters\":{\"type\":\"object\""));
        assert!(!schema.contains("\"input_schema\""));
        assert!(!schema.contains("shell_command"));
        assert!(!schema.contains("patch_apply"));
    }

    #[test]
    fn read_only_tools_are_concurrency_safe() {
        for tool in core_tool_specs()
            .into_iter()
            .filter(|tool| tool.risk == ToolRisk::ReadOnly)
        {
            assert!(
                tool.concurrency_safe,
                "{} should be safe to parallelize",
                tool.tool_id
            );
        }
    }

    #[test]
    fn non_readonly_tools_do_not_inline_large_results() {
        for tool in core_tool_specs().into_iter().filter(|tool| {
            matches!(
                tool.risk,
                ToolRisk::WritesFiles
                    | ToolRisk::ExecutesCommand
                    | ToolRisk::UsesNetwork
                    | ToolRisk::ExportsArtifact
            )
        }) {
            assert_ne!(tool.result_policy, ToolResultPolicy::Inline);
        }
    }
}
