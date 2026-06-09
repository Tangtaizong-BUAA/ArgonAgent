use crate::agent_kernel::permission_policy::{
    PermissionDecision as ModePermissionDecision, PermissionEvaluationRequest, PermissionMode,
    PermissionPolicy as ModePolicy,
};
use crate::permission_policy::{
    PermissionCheck, PermissionContext, PermissionPatternKind, PermissionRequest,
    PermissionResolution, PermissionRule, PermissionRuleDecision, PermissionRuleScope,
    PermissionRuleSet, PermissionRuleStore, ToolPermissionResult,
};
use researchcode_kernel::PermissionRequestType;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandDecision {
    Allow,
    Ask,
    AskPackageInstall,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandTokenizeError {
    DanglingEscape,
    UnclosedQuote(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandClassification {
    pub decision: CommandDecision,
    pub normalized_program: Option<String>,
    pub reasons: Vec<String>,
    pub tokens: Vec<String>,
}

const DENY_SUBSTRINGS: &[&str] = &[
    "rm -rf",
    ".env",
    "~/.ssh",
    "id_rsa",
    "id_ed25519",
    "git push",
    "git reset",
    "git clean",
    "git checkout --",
    "git filter-branch",
    "git rebase",
    "--force",
    "sudo ",
    "chmod 777",
    "/etc/passwd",
    "/etc/shadow",
    " -delete",
    " -exec ",
    " -ok ",
    " -fprint",
    " -fls",
];

const PACKAGE_INSTALL_PREFIXES: &[&[&str]] = &[
    &["npm", "install"],
    &["pnpm", "install"],
    &["yarn", "add"],
    &["pip", "install"],
    &["pip3", "install"],
    &["python", "-m", "pip", "install"],
    &["python3", "-m", "pip", "install"],
    &["cargo", "install"],
    &["uv", "add"],
    &["uv", "pip", "install"],
    &["bun", "add"],
    &["deno", "install"],
];

const HARD_DENY_PROGRAMS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "dash",
    "sudo",
    "dd",
    "fdisk",
    "systemctl",
];

const ALLOW_PREFIXES: &[&[&str]] = &[
    &["rg"],
    &["find"],
    &["ls"],
    &["wc"],
    &["cargo", "check"],
    &["cargo", "test"],
    &["python3", "-m", "unittest"],
    &["python", "-m", "unittest"],
    &["python3", "scripts/prototype_patch_validator.py"],
    &["python3", "scripts/validate_event_sequence.py"],
    &["python3", "scripts/validate_kernel_schemas.py"],
    &["npm", "test"],
    &["pytest"],
];

const DANGEROUS_FILES: &[&str] = &[
    ".env",
    ".gitconfig",
    ".gitmodules",
    ".bashrc",
    ".bash_profile",
    ".zshrc",
    ".zprofile",
    ".profile",
    ".ripgreprc",
    ".mcp.json",
    ".claude.json",
    "id_rsa",
    "id_ed25519",
    "id_ecdsa",
    "id_dsa",
    "authorized_keys",
    "known_hosts",
    "config",
    "credentials",
    ".git-credentials",
    ".netrc",
    ".npmrc",
    ".pypirc",
    "token.txt",
    "secrets.json",
    "secrets.yml",
    "secrets.yaml",
];

const DANGEROUS_DIRS: &[&str] = &[".git", ".vscode", ".idea", ".claude", ".ssh"];

const DOS_DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[derive(Debug, Clone, Default)]
pub struct DenialTracker {
    pub consecutive_denials: u32,
    pub total_denials: u32,
}

impl DenialTracker {
    pub fn record_denial(&mut self) {
        self.consecutive_denials += 1;
        self.total_denials += 1;
    }

    pub fn record_success(&mut self) {
        self.consecutive_denials = 0;
    }

    pub fn should_fallback(&self) -> bool {
        self.consecutive_denials >= 3 || self.total_denials >= 20
    }

    pub fn reset_total(&mut self) {
        self.total_denials = 0;
    }
}

pub fn classify_command(command: &str) -> CommandDecision {
    classify_command_with_reasons(command).decision
}

pub fn classify_command_with_reasons(command: &str) -> CommandClassification {
    let mut reasons = Vec::new();
    let lowered = command.to_lowercase();
    if DENY_SUBSTRINGS.iter().any(|part| lowered.contains(part)) {
        reasons.push("hard-deny substring matched".to_string());
        return classification(CommandDecision::Deny, None, reasons, Vec::new());
    }
    if [";", "&&", "||", "$(", "`", "|", ">", "<", "\n", "\r"]
        .iter()
        .any(|meta| command.contains(meta))
    {
        reasons.push("shell control operator or redirection is not allowed".to_string());
        return classification(CommandDecision::Deny, None, reasons, Vec::new());
    }
    if contains_background_operator(command) {
        reasons.push("background execution requires an explicit gated shell adapter".to_string());
        return classification(CommandDecision::Deny, None, reasons, Vec::new());
    }
    let Ok(tokens) = tokenize_command(command) else {
        reasons.push("command could not be tokenized safely".to_string());
        return classification(CommandDecision::Deny, None, reasons, Vec::new());
    };
    if tokens.is_empty() {
        reasons.push("empty command".to_string());
        return classification(CommandDecision::Deny, None, reasons, tokens);
    }
    let normalized_program = tokens.first().map(|token| token.to_ascii_lowercase());
    let normalized_program_basename = normalized_program.as_deref().map(command_program_basename);
    if let Some(program) = normalized_program_basename.as_deref() {
        if HARD_DENY_PROGRAMS.contains(&program) || program.starts_with("mkfs") {
            reasons.push("hard-denied shell/system program".to_string());
            return classification(CommandDecision::Deny, normalized_program, reasons, tokens);
        }
        if matches!(
            program,
            "curl" | "wget" | "scp" | "rsync" | "ftp" | "sftp" | "nc" | "netcat"
        ) {
            reasons.push("network transfer command is not auto-executable".to_string());
            return classification(CommandDecision::Deny, normalized_program, reasons, tokens);
        }
        if matches!(
            program,
            "rm" | "mv" | "cp" | "chmod" | "chown" | "kill" | "pkill"
        ) {
            reasons.push(
                "filesystem or process mutation requires a dedicated tool and approval".to_string(),
            );
            return classification(CommandDecision::Deny, normalized_program, reasons, tokens);
        }
    }
    if tokens.iter().any(|token| is_sensitive_token(token)) {
        reasons.push("sensitive path token matched".to_string());
        return classification(CommandDecision::Deny, normalized_program, reasons, tokens);
    }
    if tokens.iter().skip(1).any(|token| {
        token.starts_with('/') && !token.starts_with("/private/tmp") && !token.starts_with("/tmp")
    }) {
        reasons.push("absolute path outside temporary workspace requires approval".to_string());
        return classification(CommandDecision::Ask, normalized_program, reasons, tokens);
    }
    if PACKAGE_INSTALL_PREFIXES
        .iter()
        .any(|prefix| starts_with(&tokens, prefix))
    {
        reasons.push("package installation command".to_string());
        return classification(
            CommandDecision::AskPackageInstall,
            normalized_program,
            reasons,
            tokens,
        );
    }
    if ALLOW_PREFIXES
        .iter()
        .any(|prefix| starts_with(&tokens, prefix))
    {
        reasons.push("safe allowlist prefix matched".to_string());
        return classification(CommandDecision::Allow, normalized_program, reasons, tokens);
    }
    reasons.push("not in allowlist".to_string());
    classification(CommandDecision::Ask, normalized_program, reasons, tokens)
}

fn classification(
    decision: CommandDecision,
    normalized_program: Option<String>,
    reasons: Vec<String>,
    tokens: Vec<String>,
) -> CommandClassification {
    CommandClassification {
        decision,
        normalized_program,
        reasons,
        tokens,
    }
}

fn contains_background_operator(command: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in command.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
            continue;
        }
        if ch == '&' {
            return true;
        }
    }
    false
}

fn command_program_basename(program: &str) -> String {
    program
        .rsplit(|ch| ch == '/' || ch == '\\')
        .next()
        .unwrap_or(program)
        .to_string()
}

fn is_sensitive_token(token: &str) -> bool {
    let lowered = token.to_ascii_lowercase();
    lowered == ".env"
        || lowered.ends_with("/.env")
        || lowered.contains("/.env.")
        || lowered.contains("/.ssh")
        || lowered.contains("id_rsa")
        || lowered.contains("id_ed25519")
        || lowered.contains("private_key")
}

fn starts_with(tokens: &[String], prefix: &[&str]) -> bool {
    tokens.len() >= prefix.len()
        && tokens
            .iter()
            .zip(prefix.iter())
            .all(|(token, expected)| token == expected)
}

pub fn tokenize_command(command: &str) -> Result<Vec<String>, CommandTokenizeError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut token_started = false;

    for char in command.chars() {
        if escaped {
            current.push(char);
            token_started = true;
            escaped = false;
            continue;
        }
        if char == '\\' {
            escaped = true;
            token_started = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if char == active_quote {
                quote = None;
            } else {
                current.push(char);
            }
            token_started = true;
            continue;
        }
        if char == '"' || char == '\'' {
            quote = Some(char);
            token_started = true;
            continue;
        }
        if char.is_whitespace() {
            if token_started {
                tokens.push(std::mem::take(&mut current));
                token_started = false;
            }
            continue;
        }
        current.push(char);
        token_started = true;
    }
    if escaped {
        return Err(CommandTokenizeError::DanglingEscape);
    }
    if let Some(active_quote) = quote {
        return Err(CommandTokenizeError::UnclosedQuote(active_quote));
    }
    if token_started {
        tokens.push(current);
    }
    Ok(tokens)
}

pub struct ShellCommandTool;

impl PermissionCheck for ShellCommandTool {
    fn tool_id(&self) -> &str {
        "shell.command"
    }

    fn is_state_changing(&self) -> bool {
        true
    }

    fn is_file_edit(&self) -> bool {
        false
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn check_permissions(
        &self,
        args: &serde_json::Value,
        _ctx: &PermissionContext,
    ) -> ToolPermissionResult {
        let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
        if command.is_empty() {
            return ToolPermissionResult::Deny {
                reason: "empty command".to_string(),
            };
        }
        let classification = classify_command_with_reasons(command);
        match classification.decision {
            CommandDecision::Allow => ToolPermissionResult::Allow,
            CommandDecision::Deny => ToolPermissionResult::SafetyCheck {
                reason: classification.reasons.join("; "),
                classifier_approvable: false,
            },
            CommandDecision::Ask | CommandDecision::AskPackageInstall => {
                ToolPermissionResult::Ask {
                    reason: classification.reasons.join("; "),
                }
            }
        }
    }
}

pub struct FileWriteTool;

impl PermissionCheck for FileWriteTool {
    fn tool_id(&self) -> &str {
        "file.write"
    }

    fn is_state_changing(&self) -> bool {
        true
    }

    fn is_file_edit(&self) -> bool {
        true
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn check_permissions(
        &self,
        args: &serde_json::Value,
        _ctx: &PermissionContext,
    ) -> ToolPermissionResult {
        let path = args
            .get("file_path")
            .or_else(|| args.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if path.is_empty() {
            return ToolPermissionResult::Passthrough;
        }
        if let Some(reason) = check_dangerous_path(path) {
            return ToolPermissionResult::SafetyCheck {
                reason,
                classifier_approvable: true,
            };
        }
        ToolPermissionResult::Passthrough
    }
}

pub struct FileReadTool;

impl PermissionCheck for FileReadTool {
    fn tool_id(&self) -> &str {
        "file.read"
    }

    fn is_state_changing(&self) -> bool {
        false
    }

    fn is_file_edit(&self) -> bool {
        false
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

pub struct FileEditTool;

impl PermissionCheck for FileEditTool {
    fn tool_id(&self) -> &str {
        "file.edit"
    }

    fn is_state_changing(&self) -> bool {
        true
    }

    fn is_file_edit(&self) -> bool {
        true
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn check_permissions(
        &self,
        args: &serde_json::Value,
        _ctx: &PermissionContext,
    ) -> ToolPermissionResult {
        let path = args
            .get("file_path")
            .or_else(|| args.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if path.is_empty() {
            return ToolPermissionResult::Passthrough;
        }
        if let Some(reason) = check_dangerous_path(path) {
            return ToolPermissionResult::SafetyCheck {
                reason,
                classifier_approvable: true,
            };
        }
        ToolPermissionResult::Passthrough
    }
}

pub struct PatchApplyTool;

impl PermissionCheck for PatchApplyTool {
    fn tool_id(&self) -> &str {
        "patch.apply"
    }

    fn is_state_changing(&self) -> bool {
        true
    }

    fn is_file_edit(&self) -> bool {
        true
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn check_permissions(
        &self,
        args: &serde_json::Value,
        _ctx: &PermissionContext,
    ) -> ToolPermissionResult {
        let path = args
            .get("file_path")
            .or_else(|| args.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if path.is_empty() {
            return ToolPermissionResult::Passthrough;
        }
        if let Some(reason) = check_dangerous_path(path) {
            return ToolPermissionResult::SafetyCheck {
                reason,
                classifier_approvable: true,
            };
        }
        ToolPermissionResult::Passthrough
    }
}

pub struct DefaultTool {
    tool_id: String,
    is_read_only: bool,
    is_state_changing: bool,
}

impl DefaultTool {
    pub fn new(tool_id: impl Into<String>) -> Self {
        let id = tool_id.into();
        let read_only = matches!(
            id.as_str(),
            "file.read"
                | "file.list_directory"
                | "file.list_tree"
                | "repo.map"
                | "search.ripgrep"
                | "git.status"
        );
        let state_changing = matches!(
            id.as_str(),
            "file.write"
                | "file.edit"
                | "file.multi_edit"
                | "patch.apply"
                | "shell.command"
                | "powershell.command"
        );
        Self {
            tool_id: id,
            is_read_only: read_only,
            is_state_changing: state_changing,
        }
    }
}

impl PermissionCheck for DefaultTool {
    fn tool_id(&self) -> &str {
        &self.tool_id
    }

    fn is_state_changing(&self) -> bool {
        self.is_state_changing
    }

    fn is_file_edit(&self) -> bool {
        matches!(
            self.tool_id.as_str(),
            "file.write" | "file.edit" | "file.multi_edit" | "patch.apply"
        )
    }

    fn is_read_only(&self) -> bool {
        self.is_read_only
    }
}

pub fn check_dangerous_path(path: &str) -> Option<String> {
    if path.starts_with("\\\\")
        || path.starts_with("//")
        || path.starts_with("\\\\?\\")
        || path.starts_with("\\\\.\\")
        || path.starts_with("//?/")
        || path.starts_with("//./")
    {
        return Some("unsupported absolute or device path is not allowed".to_string());
    }
    if path.ends_with('.') || path.ends_with(' ') {
        return Some("path with trailing dot or space is suspicious".to_string());
    }
    let normalized = path.replace('\\', "/");
    let segments: Vec<&str> = normalized.split('/').collect();
    for (index, segment) in segments.iter().enumerate() {
        let lower = segment.to_lowercase();
        if *segment == ".." {
            return Some("path traversal with '..' not allowed".to_string());
        }
        if segment.contains(':')
            && (index > 0 || !matches!(segment.len(), 2 | 3) || !segment.ends_with(':'))
        {
            return Some("NTFS alternate data streams not allowed".to_string());
        }
        if segment.contains('~') && segment.chars().any(|character| character.is_ascii_digit()) {
            return Some("8.3 short names not allowed".to_string());
        }
        if DOS_DEVICE_NAMES
            .iter()
            .any(|device| lower == device.to_lowercase())
        {
            return Some(format!("DOS device name '{segment}' not allowed"));
        }
        if DANGEROUS_DIRS.iter().any(|dir| dir.to_lowercase() == lower) {
            if lower == ".claude"
                && segments
                    .get(index + 1)
                    .map(|item| item.to_lowercase())
                    .as_deref()
                    == Some("worktrees")
            {
                continue;
            }
            return Some(format!(
                "path inside '{segment}' directory requires approval"
            ));
        }
        if index == segments.len() - 1
            && DANGEROUS_FILES
                .iter()
                .any(|file| file.to_lowercase() == lower)
        {
            return Some(format!("'{segment}' requires approval for automatic edits"));
        }
    }
    if (normalized.contains("/...")
        || normalized.contains("./...")
        || normalized.starts_with("..."))
        && !normalized.contains("[...")
    {
        return Some("path with consecutive dots is suspicious".to_string());
    }
    None
}

pub(crate) fn request_type_for_tool(tool_id: &str) -> PermissionRequestType {
    match tool_id {
        "shell.command" | "powershell.command" => PermissionRequestType::Command,
        "file.write" | "file.edit" | "file.multi_edit" | "patch.apply" => {
            PermissionRequestType::FileWrite
        }
        _ => PermissionRequestType::ProtectedPath,
    }
}

fn find_rule_in_policy(
    policy: &PermissionRuleSet,
    request_type: &PermissionRequestType,
    tool_id: &str,
    normalized_summary: &str,
    decision: &PermissionRuleDecision,
) -> Option<PermissionRule> {
    policy
        .rules
        .iter()
        .find(|rule| {
            &rule.decision == decision && rule.matches(request_type, tool_id, normalized_summary)
        })
        .cloned()
}

/// Long-lived permission entry point for doc39 convergence.
///
/// Runtime paths hold this gate across multiple evaluations so denial tracking
/// is not reset on every tool call.
#[derive(Debug, Clone)]
pub struct PermissionGate {
    policy_store: Arc<PermissionRuleStore>,
    inline_policy: PermissionRuleSet,
    mode: PermissionMode,
    denial_tracker: DenialTracker,
    workspace_root: String,
    session_id: String,
}

impl PermissionGate {
    pub fn new(
        policy_store: Arc<PermissionRuleStore>,
        inline_policy: PermissionRuleSet,
        mode: PermissionMode,
        workspace_root: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            policy_store,
            inline_policy,
            mode,
            denial_tracker: DenialTracker::default(),
            workspace_root: workspace_root.into(),
            session_id: session_id.into(),
        }
    }

    pub fn evaluate(
        &mut self,
        request: PermissionRequest<'_>,
        tool: &dyn PermissionCheck,
    ) -> PermissionResolution {
        self.mode = request.mode;
        let tool_id = request.tool_id;
        let request_type = request.request_type.clone();
        let command_summary = request.command_summary;

        if let Some(rule) = self.find_rule(
            &request_type,
            tool_id,
            command_summary,
            PermissionRuleDecision::Deny,
        ) {
            self.denial_tracker.record_denial();
            return PermissionResolution::Deny {
                reason: rule.reason.clone(),
            };
        }

        if let Some(rule) = self.find_rule(
            &request_type,
            tool_id,
            command_summary,
            PermissionRuleDecision::Ask,
        ) {
            return PermissionResolution::Ask {
                force: true,
                reason: rule.reason.clone(),
                safety: false,
                persistable: true,
                suggested_rule: None,
            };
        }

        let ctx = PermissionContext {
            workspace_root: Some(self.workspace_root.clone()),
            session_id: request.session_id.to_string(),
        };
        let tool_result = tool.check_permissions(request.args, &ctx);

        if let ToolPermissionResult::Deny { reason } = &tool_result {
            self.denial_tracker.record_denial();
            return PermissionResolution::Deny {
                reason: reason.clone(),
            };
        }

        if tool.requires_user_interaction()
            && matches!(tool_result, ToolPermissionResult::Ask { .. })
        {
            return PermissionResolution::Ask {
                force: true,
                reason: "tool requires user interaction".to_string(),
                safety: false,
                persistable: false,
                suggested_rule: None,
            };
        }

        if let ToolPermissionResult::Ask { reason: ask_reason } = &tool_result {
            if let Some(rule) = self.find_rule(
                &request_type,
                tool_id,
                command_summary,
                PermissionRuleDecision::Ask,
            ) {
                return PermissionResolution::Ask {
                    force: true,
                    reason: rule.reason.clone(),
                    safety: false,
                    persistable: true,
                    suggested_rule: Some(rule.clone()),
                };
            }
            return PermissionResolution::Ask {
                force: false,
                reason: ask_reason.clone(),
                safety: false,
                persistable: true,
                suggested_rule: None,
            };
        }

        if let ToolPermissionResult::SafetyCheck {
            reason,
            classifier_approvable,
        } = &tool_result
        {
            if !classifier_approvable {
                self.denial_tracker.record_denial();
                return PermissionResolution::Deny {
                    reason: reason.clone(),
                };
            }
            return PermissionResolution::Ask {
                force: true,
                reason: reason.clone(),
                safety: true,
                persistable: *classifier_approvable,
                suggested_rule: None,
            };
        }

        if self.mode == PermissionMode::BypassPermissions {
            self.denial_tracker.record_success();
            return PermissionResolution::Allow;
        }

        if let Some(_rule) = self.find_rule(
            &request_type,
            tool_id,
            command_summary,
            PermissionRuleDecision::Allow,
        ) {
            self.denial_tracker.record_success();
            return PermissionResolution::Allow;
        }

        let result = self.resolve_by_mode(tool, command_summary);
        match &result {
            PermissionResolution::Deny { .. } => self.denial_tracker.record_denial(),
            PermissionResolution::Allow => self.denial_tracker.record_success(),
            _ => {}
        }
        result
    }

    pub fn evaluate_current(
        &mut self,
        tool_id: &str,
        args: &serde_json::Value,
        request_type: PermissionRequestType,
        command_summary: Option<&str>,
        tool: &dyn PermissionCheck,
    ) -> PermissionResolution {
        let session_id = self.session_id.clone();
        self.evaluate(
            PermissionRequest {
                mode: self.mode,
                tool_id,
                args,
                request_type,
                session_id: &session_id,
                command_summary,
            },
            tool,
        )
    }

    pub fn denial_count(&self) -> u32 {
        self.denial_tracker.total_denials
    }

    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    pub fn consecutive_denials(&self) -> u32 {
        self.denial_tracker.consecutive_denials
    }

    fn resolve_by_mode(
        &self,
        tool: &dyn PermissionCheck,
        subcommand: Option<&str>,
    ) -> PermissionResolution {
        let tool_id = tool.tool_id();
        let fallback_request = PermissionEvaluationRequest {
            mode: self.mode,
            tool_id,
            args: &serde_json::Value::Null,
            request_type: request_type_for_tool(tool_id),
            session_id: &self.session_id,
            command_summary: subcommand,
        };
        match ModePolicy::evaluate(&fallback_request) {
            ModePermissionDecision::Allow => PermissionResolution::Allow,
            ModePermissionDecision::Deny { reason } => PermissionResolution::Deny { reason },
            ModePermissionDecision::Ask { reason } => PermissionResolution::Ask {
                force: false,
                reason,
                safety: false,
                persistable: true,
                suggested_rule: self.build_suggested_rule(tool_id, subcommand),
            },
        }
    }

    fn find_rule(
        &self,
        request_type: &PermissionRequestType,
        tool_id: &str,
        subcommand: Option<&str>,
        decision: PermissionRuleDecision,
    ) -> Option<PermissionRule> {
        let normalized = subcommand.unwrap_or("");
        find_rule_in_policy(
            &self.inline_policy,
            request_type,
            tool_id,
            normalized,
            &decision,
        )
        .or_else(|| {
            let policy = self.policy_store.load().ok()?;
            find_rule_in_policy(&policy, request_type, tool_id, normalized, &decision)
        })
    }

    fn build_suggested_rule(
        &self,
        tool_id: &str,
        subcommand: Option<&str>,
    ) -> Option<PermissionRule> {
        let pattern = subcommand.unwrap_or("").to_string();
        if pattern.is_empty() {
            return None;
        }
        Some(PermissionRule {
            rule_id: format!("suggested_{tool_id}_{}", pattern.replace(' ', "_")),
            scope: PermissionRuleScope::Project,
            request_type: request_type_for_tool(tool_id),
            tool_id: tool_id.to_string(),
            pattern_kind: PermissionPatternKind::Exact,
            pattern,
            decision: PermissionRuleDecision::Allow,
            reason: "user approved".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission_policy::{PermissionContext, PermissionRuleStore, ToolPermissionResult};
    use researchcode_kernel::PermissionRequestType;

    fn request<'a>(
        mode: PermissionMode,
        tool_id: &'a str,
        args: &'a serde_json::Value,
    ) -> PermissionRequest<'a> {
        PermissionRequest {
            mode,
            tool_id,
            args,
            request_type: PermissionRequestType::FileWrite,
            session_id: "session",
            command_summary: None,
        }
    }

    #[test]
    fn gate_keeps_denial_tracker_across_evaluations() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(PermissionRuleStore::new(temp.path().join("policy.tsv")));
        let mut gate = PermissionGate::new(
            store,
            PermissionRuleSet::default(),
            PermissionMode::Plan,
            temp.path().to_string_lossy(),
            "session",
        );
        let args = serde_json::json!({"path":"src/main.rs"});
        for _ in 0..2 {
            assert!(matches!(
                gate.evaluate(
                    request(PermissionMode::Plan, "file.write", &args),
                    &FileWriteTool
                ),
                PermissionResolution::Deny { .. }
            ));
        }
        assert_eq!(gate.denial_count(), 2);
        assert_eq!(gate.consecutive_denials(), 2);
    }

    #[test]
    fn gate_resets_consecutive_denials_after_allow() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(PermissionRuleStore::new(temp.path().join("policy.tsv")));
        let mut gate = PermissionGate::new(
            store,
            PermissionRuleSet::default(),
            PermissionMode::Plan,
            temp.path().to_string_lossy(),
            "session",
        );
        let write_args = serde_json::json!({"path":"src/main.rs"});
        assert!(matches!(
            gate.evaluate(
                request(PermissionMode::Plan, "file.write", &write_args),
                &FileWriteTool
            ),
            PermissionResolution::Deny { .. }
        ));

        let read_args = serde_json::json!({"path":"src/main.rs"});
        let read_request = PermissionRequest {
            mode: PermissionMode::Plan,
            tool_id: "file.read",
            args: &read_args,
            request_type: PermissionRequestType::ProtectedPath,
            session_id: "session",
            command_summary: None,
        };
        assert_eq!(
            gate.evaluate(read_request, &FileReadTool),
            PermissionResolution::Allow
        );
        assert_eq!(gate.denial_count(), 1);
        assert_eq!(gate.consecutive_denials(), 0);
    }

    #[test]
    fn dangerous_path_classifier_preserves_sensitive_path_guards() {
        assert!(check_dangerous_path(".env").is_some());
        assert!(check_dangerous_path(".git/config").is_some());
        assert!(check_dangerous_path("\\\\server\\share").is_some());
        assert!(check_dangerous_path("../../../etc/passwd").is_some());
        assert!(check_dangerous_path(".ssh/id_rsa").is_some());
        assert!(check_dangerous_path("app/[id]/[...slug]/page.tsx").is_none());
        assert!(check_dangerous_path(".claude/worktrees/test").is_none());
        assert!(check_dangerous_path("src/main.rs").is_none());
    }

    #[test]
    fn shell_interpreters_and_system_mutators_are_hard_denied() {
        for command in [
            "sh -c ls",
            "bash -lc ls",
            "zsh -lc ls",
            "dash -c ls",
            "sudo ls",
            "dd if=/dev/zero of=disk.img",
            "mkfs.ext4 /dev/disk1",
            "fdisk -l",
            "systemctl restart service",
            "/bin/sh -c ls",
            "/bin/bash -lc ls",
            "/usr/bin/sudo ls",
            "/sbin/mkfs.ext4 /dev/disk1",
            "./bash -lc ls",
        ] {
            let classification = classify_command_with_reasons(command);
            assert_eq!(
                classification.decision,
                CommandDecision::Deny,
                "{command} should be denied"
            );
            assert!(
                classification
                    .reasons
                    .iter()
                    .any(|reason| reason.contains("hard-deny")
                        || reason.contains("hard-denied")
                        || reason.contains("shell control")
                        || reason.contains("absolute path")),
                "{command} reasons were {:?}",
                classification.reasons
            );
        }
    }

    #[test]
    fn permission_gate_maps_non_approvable_command_classifier_to_deny() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(PermissionRuleStore::new(temp.path().join("policy.tsv")));
        let mut gate = PermissionGate::new(
            store,
            PermissionRuleSet::default(),
            PermissionMode::Default,
            temp.path().to_string_lossy(),
            "session",
        );
        let args = serde_json::json!({"command":"/bin/bash -lc ls"});
        let request = PermissionRequest {
            mode: PermissionMode::Default,
            tool_id: "shell.command",
            args: &args,
            request_type: PermissionRequestType::Command,
            session_id: "session",
            command_summary: Some("/bin/bash -lc ls"),
        };
        assert!(matches!(
            gate.evaluate(request, &ShellCommandTool),
            PermissionResolution::Deny { .. }
        ));
    }

    struct MatrixTool {
        tool_id: &'static str,
        is_file_edit: bool,
    }

    impl PermissionCheck for MatrixTool {
        fn tool_id(&self) -> &str {
            self.tool_id
        }

        fn is_state_changing(&self) -> bool {
            true
        }

        fn is_file_edit(&self) -> bool {
            self.is_file_edit
        }

        fn is_read_only(&self) -> bool {
            false
        }

        fn check_permissions(
            &self,
            _args: &serde_json::Value,
            _ctx: &PermissionContext,
        ) -> ToolPermissionResult {
            ToolPermissionResult::Passthrough
        }
    }

    #[test]
    fn permission_gate_covers_every_mode_and_request_type_matrix() {
        let modes = [
            PermissionMode::BypassPermissions,
            PermissionMode::Plan,
            PermissionMode::AcceptEdits,
            PermissionMode::DontAsk,
            PermissionMode::Default,
        ];
        let request_types = [
            PermissionRequestType::Command,
            PermissionRequestType::FileWrite,
            PermissionRequestType::Network,
            PermissionRequestType::PackageInstall,
            PermissionRequestType::CloudModel,
            PermissionRequestType::ProtectedPath,
            PermissionRequestType::ArtifactExport,
        ];
        let args = serde_json::json!({"path":"src/main.rs","command":"rg TODO ."});

        for mode in modes {
            for request_type in request_types.clone() {
                let temp = tempfile::tempdir().unwrap();
                let store = Arc::new(PermissionRuleStore::new(temp.path().join("policy.tsv")));
                let mut gate = PermissionGate::new(
                    store,
                    PermissionRuleSet::default(),
                    mode,
                    temp.path().to_string_lossy(),
                    "session",
                );
                let (tool_id, is_file_edit, is_state_changing, is_read_only) =
                    matrix_tool_for_request_type(&request_type);
                let tool = MatrixTool {
                    tool_id,
                    is_file_edit,
                };
                let request = PermissionRequest {
                    mode,
                    tool_id,
                    args: &args,
                    request_type: request_type.clone(),
                    session_id: "session",
                    command_summary: Some("matrix"),
                };
                let decision = gate.evaluate(request, &tool);

                match mode {
                    PermissionMode::BypassPermissions => {
                        assert_eq!(decision, PermissionResolution::Allow)
                    }
                    PermissionMode::Plan if is_state_changing => {
                        assert!(matches!(decision, PermissionResolution::Deny { .. }))
                    }
                    PermissionMode::Plan => assert_eq!(decision, PermissionResolution::Allow),
                    PermissionMode::AcceptEdits if is_file_edit => {
                        assert_eq!(decision, PermissionResolution::Allow)
                    }
                    PermissionMode::AcceptEdits if is_state_changing => {
                        assert!(matches!(decision, PermissionResolution::Ask { .. }))
                    }
                    PermissionMode::AcceptEdits => {
                        assert_eq!(decision, PermissionResolution::Allow)
                    }
                    PermissionMode::DontAsk if is_read_only => {
                        assert_eq!(decision, PermissionResolution::Allow)
                    }
                    PermissionMode::DontAsk => {
                        assert!(matches!(decision, PermissionResolution::Deny { .. }))
                    }
                    PermissionMode::Default if is_state_changing => {
                        assert!(matches!(decision, PermissionResolution::Ask { .. }))
                    }
                    PermissionMode::Default => assert_eq!(decision, PermissionResolution::Allow),
                }
            }
        }
    }

    fn matrix_tool_for_request_type(
        request_type: &PermissionRequestType,
    ) -> (&'static str, bool, bool, bool) {
        match request_type {
            PermissionRequestType::Command | PermissionRequestType::PackageInstall => {
                ("shell.command", false, true, false)
            }
            PermissionRequestType::FileWrite | PermissionRequestType::ProtectedPath => {
                ("file.write", true, true, false)
            }
            PermissionRequestType::Network => ("network.call", false, false, false),
            PermissionRequestType::CloudModel => ("cloud.model", false, false, false),
            PermissionRequestType::ArtifactExport => ("artifact.export", false, false, false),
        }
    }
}
