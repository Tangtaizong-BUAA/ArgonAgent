//! Shell command permission and execution boundary.
//!
//! Commands are tokenized and executed without a shell. Shell control operators,
//! redirection, command substitution, destructive commands, and sensitive path
//! access stay blocked before execution.

use crate::agent_kernel::permission_gate::{
    classify_command_with_reasons, tokenize_command, CommandDecision,
};
use crate::artifact::{ArtifactKind, ArtifactRecord, ArtifactStore};
use crate::secret_scan::redact_text_for_secrets;
use researchcode_kernel::PermissionDecisionKind;
use std::io;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRequest {
    pub command: String,
    pub cwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandExecutionPlan {
    pub request: CommandRequest,
    pub classifier_decision: CommandDecision,
    pub classifier_reasons: Vec<String>,
    pub requires_permission: bool,
    pub blocked: bool,
    pub normalized_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAuthorization {
    AllowedToRun,
    RequiresPermission,
    BlockedByPolicy,
    DeniedByUser,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandRunError {
    NotAuthorized(CommandAuthorization),
    EmptyCommand,
    InvalidCommand(String),
    Io(String),
}

pub fn prepare_command(request: CommandRequest) -> CommandExecutionPlan {
    let classification = classify_command_with_reasons(&request.command);
    let classifier_decision = classification.decision.clone();
    let requires_permission = matches!(
        classifier_decision,
        CommandDecision::Ask | CommandDecision::AskPackageInstall
    );
    let blocked = classifier_decision == CommandDecision::Deny;
    let normalized_summary = format!(
        "Run `{}` in `{}` decision={:?} reasons={}",
        request.command,
        request.cwd,
        classifier_decision,
        classification.reasons.join("; ")
    );
    CommandExecutionPlan {
        request,
        classifier_decision,
        classifier_reasons: classification.reasons,
        requires_permission,
        blocked,
        normalized_summary,
    }
}

pub fn authorize_command(
    plan: &CommandExecutionPlan,
    permission_decision: Option<PermissionDecisionKind>,
) -> CommandAuthorization {
    if plan.blocked {
        return CommandAuthorization::BlockedByPolicy;
    }
    if !plan.requires_permission {
        return CommandAuthorization::AllowedToRun;
    }
    match permission_decision {
        Some(PermissionDecisionKind::AllowOnce)
        | Some(PermissionDecisionKind::AllowSession)
        | Some(PermissionDecisionKind::AllowProjectRule) => CommandAuthorization::AllowedToRun,
        Some(PermissionDecisionKind::Deny) | Some(PermissionDecisionKind::Modify) => {
            CommandAuthorization::DeniedByUser
        }
        None => CommandAuthorization::RequiresPermission,
    }
}

pub fn capture_command_output_artifact(
    store: &ArtifactStore,
    artifact_id: impl Into<String>,
    output: &CommandOutput,
) -> Result<ArtifactRecord, io::Error> {
    let stdout = redact_text_for_secrets(&output.stdout);
    let stderr = redact_text_for_secrets(&output.stderr);
    let payload = format!(
        "{{\"command\":\"{}\",\"exit_code\":{},\"stdout\":\"{}\",\"stderr\":\"{}\"}}",
        escape(&output.command),
        output.exit_code,
        escape(&stdout),
        escape(&stderr)
    );
    store.put_bytes_auto_hash(
        artifact_id,
        ArtifactKind::CommandOutput,
        "internal",
        payload.as_bytes(),
    )
}

pub fn run_prepared_command(
    plan: &CommandExecutionPlan,
    permission_decision: Option<PermissionDecisionKind>,
) -> Result<CommandOutput, CommandRunError> {
    let authorization = authorize_command(plan, permission_decision);
    if authorization != CommandAuthorization::AllowedToRun {
        return Err(CommandRunError::NotAuthorized(authorization));
    }
    let tokens = tokenize_command(&plan.request.command)
        .map_err(|error| CommandRunError::InvalidCommand(format!("{error:?}")))?;
    let Some(program) = tokens.first() else {
        return Err(CommandRunError::EmptyCommand);
    };
    let output = Command::new(program)
        .args(tokens.iter().skip(1))
        .current_dir(&plan.request.cwd)
        .output()
        .map_err(|error| CommandRunError::Io(error.to_string()))?;
    Ok(CommandOutput {
        command: plan.request.command.clone(),
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn escape(value: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_command_can_run_without_permission() {
        let plan = prepare_command(CommandRequest {
            command: "cargo test --workspace".to_string(),
            cwd: ".".to_string(),
        });
        assert_eq!(plan.classifier_decision, CommandDecision::Allow);
        assert_eq!(
            authorize_command(&plan, None),
            CommandAuthorization::AllowedToRun
        );
    }

    #[test]
    fn package_install_requires_permission() {
        let plan = prepare_command(CommandRequest {
            command: "npm install".to_string(),
            cwd: ".".to_string(),
        });
        assert!(plan.requires_permission);
        assert_eq!(
            authorize_command(&plan, None),
            CommandAuthorization::RequiresPermission
        );
        assert_eq!(
            authorize_command(&plan, Some(PermissionDecisionKind::AllowOnce)),
            CommandAuthorization::AllowedToRun
        );
    }

    #[test]
    fn dangerous_command_never_runs_even_with_permission() {
        let plan = prepare_command(CommandRequest {
            command: "rm -rf .".to_string(),
            cwd: ".".to_string(),
        });
        assert!(plan.blocked);
        assert_eq!(
            authorize_command(&plan, Some(PermissionDecisionKind::AllowOnce)),
            CommandAuthorization::BlockedByPolicy
        );
    }

    #[test]
    fn safe_prepared_command_executes_without_shell() {
        let plan = prepare_command(CommandRequest {
            command: "find . -maxdepth 0".to_string(),
            cwd: std::env::temp_dir().to_string_lossy().to_string(),
        });
        assert_eq!(plan.classifier_decision, CommandDecision::Allow);
        let output = run_prepared_command(&plan, None).unwrap();
        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains('.'));
    }

    #[test]
    fn quoted_arguments_are_preserved_without_shell() {
        let root = std::env::temp_dir().join("researchcode-command-quoted-args");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("sample.txt"), "hello world\n").unwrap();
        let plan = prepare_command(CommandRequest {
            command: "rg \"hello world\" sample.txt".to_string(),
            cwd: root.to_string_lossy().to_string(),
        });
        assert_eq!(plan.classifier_decision, CommandDecision::Allow);
        let output = run_prepared_command(&plan, None).unwrap();
        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("hello world"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn shell_redirection_is_blocked_even_with_permission() {
        let plan = prepare_command(CommandRequest {
            command: "python3 -m unittest > out.txt".to_string(),
            cwd: ".".to_string(),
        });
        assert_eq!(plan.classifier_decision, CommandDecision::Deny);
        assert_eq!(
            run_prepared_command(&plan, Some(PermissionDecisionKind::AllowOnce)),
            Err(CommandRunError::NotAuthorized(
                CommandAuthorization::BlockedByPolicy
            ))
        );
    }

    #[test]
    fn denied_prepared_command_does_not_execute() {
        let plan = prepare_command(CommandRequest {
            command: "rm -rf .".to_string(),
            cwd: ".".to_string(),
        });
        assert_eq!(
            run_prepared_command(&plan, Some(PermissionDecisionKind::AllowOnce)),
            Err(CommandRunError::NotAuthorized(
                CommandAuthorization::BlockedByPolicy
            ))
        );
    }

    #[test]
    fn captures_command_output_as_artifact() {
        let root = std::env::temp_dir().join("researchcode-command-output-artifact");
        let store = ArtifactStore::new(&root);
        let record = capture_command_output_artifact(
            &store,
            "cmd_out_1",
            &CommandOutput {
                command: "cargo test".to_string(),
                exit_code: 0,
                stdout: "ok".to_string(),
                stderr: "".to_string(),
            },
        )
        .unwrap();
        assert_eq!(record.kind, ArtifactKind::CommandOutput);
        assert_eq!(
            store
                .read_bytes(&record)
                .unwrap()
                .starts_with(b"{\"command\""),
            true
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn command_output_artifact_redacts_secret_like_tokens() {
        let root = std::env::temp_dir().join("researchcode-command-output-redaction");
        let store = ArtifactStore::new(&root);
        let record = capture_command_output_artifact(
            &store,
            "cmd_out_secret",
            &CommandOutput {
                command: "test".to_string(),
                exit_code: 0,
                stdout: "token sk-testsecret123456789".to_string(),
                stderr: "path .env".to_string(),
            },
        )
        .unwrap();
        let payload = String::from_utf8(store.read_bytes(&record).unwrap()).unwrap();
        assert!(payload.contains("[REDACTED_SECRET]"));
        assert!(payload.contains("[REDACTED_PATH]"));
        assert!(!payload.contains("sk-testsecret"));
        let _ = std::fs::remove_dir_all(root);
    }
}
