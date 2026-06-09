//! File-backed permission policy rules.
//!
//! The runtime keeps plan approval and security permission separate. This
//! module only covers security permission decisions that can be audited and
//! replayed by TUI/GUI clients.

use researchcode_kernel::{PermissionDecisionKind, PermissionRequestType};
use std::fs;
use std::path::{Path, PathBuf};

use crate::agent_kernel::permission_policy::PermissionMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionRuleScope {
    Session,
    Project,
    Global,
}

impl PermissionRuleScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Project => "project",
            Self::Global => "global",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "session" => Some(Self::Session),
            "project" => Some(Self::Project),
            "global" => Some(Self::Global),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionRuleDecision {
    Allow,
    Deny,
    Ask,
}

impl PermissionRuleDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Ask => "ask",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            "ask" => Some(Self::Ask),
            _ => None,
        }
    }

    pub fn from_permission_decision(value: &PermissionDecisionKind) -> Option<Self> {
        match value {
            PermissionDecisionKind::AllowOnce
            | PermissionDecisionKind::AllowSession
            | PermissionDecisionKind::AllowProjectRule => Some(Self::Allow),
            PermissionDecisionKind::Deny => Some(Self::Deny),
            PermissionDecisionKind::Modify => Some(Self::Ask),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionPatternKind {
    Exact,
    Prefix,
}

impl PermissionPatternKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Prefix => "prefix",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "exact" => Some(Self::Exact),
            "prefix" => Some(Self::Prefix),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRule {
    pub rule_id: String,
    pub scope: PermissionRuleScope,
    pub request_type: PermissionRequestType,
    pub tool_id: String,
    pub pattern_kind: PermissionPatternKind,
    pub pattern: String,
    pub decision: PermissionRuleDecision,
    pub reason: String,
}

impl PermissionRule {
    pub fn matches(
        &self,
        request_type: &PermissionRequestType,
        tool_id: &str,
        normalized_summary: &str,
    ) -> bool {
        if &self.request_type != request_type || self.tool_id != tool_id {
            return false;
        }
        match self.pattern_kind {
            PermissionPatternKind::Exact => self.pattern == normalized_summary,
            PermissionPatternKind::Prefix => normalized_summary.starts_with(&self.pattern),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PermissionRuleSet {
    pub rules: Vec<PermissionRule>,
}

impl PermissionRuleSet {
    pub fn add_or_replace(&mut self, rule: PermissionRule) {
        self.rules.retain(|item| item.rule_id != rule.rule_id);
        self.rules.push(rule);
        self.rules
            .sort_by(|left, right| left.rule_id.cmp(&right.rule_id));
    }

    pub fn find_match(
        &self,
        request_type: &PermissionRequestType,
        tool_id: &str,
        normalized_summary: &str,
    ) -> Option<&PermissionRule> {
        self.rules
            .iter()
            .find(|rule| rule.matches(request_type, tool_id, normalized_summary))
    }

    pub fn to_tsv(&self) -> String {
        let mut lines = vec![
            "rule_id\tscope\trequest_type\ttool_id\tpattern_kind\tpattern\tdecision\treason"
                .to_string(),
        ];
        for rule in &self.rules {
            lines.push(format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                escape_field(&rule.rule_id),
                rule.scope.as_str(),
                request_type_to_str(&rule.request_type),
                escape_field(&rule.tool_id),
                rule.pattern_kind.as_str(),
                escape_field(&rule.pattern),
                rule.decision.as_str(),
                escape_field(&rule.reason)
            ));
        }
        lines.join("\n")
    }

    pub fn from_tsv(input: &str) -> Result<Self, String> {
        let mut policy = PermissionRuleSet::default();
        for (index, line) in input.lines().enumerate() {
            if index == 0 && line.starts_with("rule_id\t") {
                continue;
            }
            if line.trim().is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 8 {
                return Err(format!(
                    "permission policy line {} has {} fields",
                    index + 1,
                    fields.len()
                ));
            }
            let request_type = PermissionRequestType::parse(fields[2])
                .ok_or_else(|| format!("unknown request_type on line {}", index + 1))?;
            policy.add_or_replace(PermissionRule {
                rule_id: unescape_field(fields[0]),
                scope: PermissionRuleScope::parse(fields[1])
                    .ok_or_else(|| format!("unknown scope on line {}", index + 1))?,
                request_type,
                tool_id: unescape_field(fields[3]),
                pattern_kind: PermissionPatternKind::parse(fields[4])
                    .ok_or_else(|| format!("unknown pattern_kind on line {}", index + 1))?,
                pattern: unescape_field(fields[5]),
                decision: PermissionRuleDecision::parse(fields[6])
                    .ok_or_else(|| format!("unknown decision on line {}", index + 1))?,
                reason: unescape_field(fields[7]),
            });
        }
        Ok(policy)
    }
}

#[derive(Debug, Clone)]
pub struct PermissionRuleStore {
    path: PathBuf,
}

impl PermissionRuleStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<PermissionRuleSet, String> {
        if !self.path.exists() {
            return Ok(PermissionRuleSet::default());
        }
        let text = fs::read_to_string(&self.path).map_err(|error| error.to_string())?;
        PermissionRuleSet::from_tsv(&text)
    }

    pub fn save(&self, policy: &PermissionRuleSet) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&self.path, policy.to_tsv()).map_err(|error| error.to_string())
    }

    pub fn add_rule(&self, rule: PermissionRule) -> Result<(), String> {
        let mut policy = self.load()?;
        policy.add_or_replace(rule);
        self.save(&policy)
    }
}

/// Backwards-compatible alias while callers migrate to the clearer rule-set name.
pub type PermissionPolicy = PermissionRuleSet;
pub type PermissionPolicyStore = PermissionRuleStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPermissionResult {
    Passthrough,
    Allow,
    Deny {
        reason: String,
    },
    Ask {
        reason: String,
    },
    SafetyCheck {
        reason: String,
        classifier_approvable: bool,
    },
}

impl ToolPermissionResult {
    pub fn is_deny(&self) -> bool {
        matches!(self, Self::Deny { .. })
    }

    pub fn is_safety_check(&self) -> bool {
        matches!(self, Self::SafetyCheck { .. })
    }
}

pub struct PermissionContext {
    pub workspace_root: Option<String>,
    pub session_id: String,
}

pub trait PermissionCheck {
    fn tool_id(&self) -> &str;
    fn requires_user_interaction(&self) -> bool {
        false
    }
    fn is_state_changing(&self) -> bool;
    fn is_file_edit(&self) -> bool;
    fn is_read_only(&self) -> bool;
    fn check_permissions(
        &self,
        args: &serde_json::Value,
        ctx: &PermissionContext,
    ) -> ToolPermissionResult {
        let _ = (args, ctx);
        ToolPermissionResult::Passthrough
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionResolution {
    Allow,
    Deny {
        reason: String,
    },
    Ask {
        force: bool,
        reason: String,
        safety: bool,
        persistable: bool,
        suggested_rule: Option<PermissionRule>,
    },
}

/// Doc39-shaped permission evaluation request.
///
/// Keep all call-specific permission inputs in one object so the runtime,
/// native loop, and future GUI replay path share the same evaluation contract.
#[derive(Debug, Clone)]
pub struct PermissionRequest<'a> {
    pub mode: PermissionMode,
    pub tool_id: &'a str,
    pub args: &'a serde_json::Value,
    pub request_type: PermissionRequestType,
    pub session_id: &'a str,
    pub command_summary: Option<&'a str>,
}

pub fn permission_rule_from_decision(
    rule_id: impl Into<String>,
    scope: PermissionRuleScope,
    request_type: PermissionRequestType,
    tool_id: impl Into<String>,
    normalized_summary: impl Into<String>,
    decision: PermissionDecisionKind,
    reason: impl Into<String>,
) -> Option<PermissionRule> {
    let rule_decision = PermissionRuleDecision::from_permission_decision(&decision)?;
    Some(PermissionRule {
        rule_id: rule_id.into(),
        scope,
        request_type,
        tool_id: tool_id.into(),
        pattern_kind: PermissionPatternKind::Exact,
        pattern: normalized_summary.into(),
        decision: rule_decision,
        reason: reason.into(),
    })
}

fn request_type_to_str(value: &PermissionRequestType) -> &'static str {
    match value {
        PermissionRequestType::Command => "command",
        PermissionRequestType::FileWrite => "file_write",
        PermissionRequestType::Network => "network",
        PermissionRequestType::PackageInstall => "package_install",
        PermissionRequestType::CloudModel => "cloud_model",
        PermissionRequestType::ProtectedPath => "protected_path",
        PermissionRequestType::ArtifactExport => "artifact_export",
    }
}

fn escape_field(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
}

fn unescape_field(value: &str) -> String {
    let mut output = String::new();
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            match ch {
                't' => output.push('\t'),
                'n' => output.push('\n'),
                '\\' => output.push('\\'),
                other => {
                    output.push('\\');
                    output.push(other);
                }
            }
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
        } else {
            output.push(ch);
        }
    }
    if escaped {
        output.push('\\');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn policy_round_trips_and_matches_exact_rule() {
        let mut policy = PermissionRuleSet::default();
        policy.add_or_replace(PermissionRule {
            rule_id: "rule_1".to_string(),
            scope: PermissionRuleScope::Project,
            request_type: PermissionRequestType::Command,
            tool_id: "shell.command".to_string(),
            pattern_kind: PermissionPatternKind::Exact,
            pattern: "command: cargo test".to_string(),
            decision: PermissionRuleDecision::Allow,
            reason: "project test command".to_string(),
        });
        let text = policy.to_tsv();
        let parsed = PermissionRuleSet::from_tsv(&text).unwrap();
        let matched = parsed
            .find_match(
                &PermissionRequestType::Command,
                "shell.command",
                "command: cargo test",
            )
            .unwrap();
        assert_eq!(matched.decision, PermissionRuleDecision::Allow);
    }

    #[test]
    fn store_persists_policy_file() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir()
            .join(format!("researchcode-permission-policy-{nonce}"))
            .join("policy.tsv");
        let store = PermissionRuleStore::new(&path);
        store
            .add_rule(
                permission_rule_from_decision(
                    "rule_shell_test",
                    PermissionRuleScope::Project,
                    PermissionRequestType::Command,
                    "shell.command",
                    "command: cargo test",
                    PermissionDecisionKind::AllowProjectRule,
                    "user approved project rule",
                )
                .unwrap(),
            )
            .unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.rules.len(), 1);
        assert_eq!(loaded.rules[0].scope, PermissionRuleScope::Project);
        let _ = std::fs::remove_file(path);
    }
}
