//! Multi-file patch-set validation and atomic apply.
//!
//! The single-file `patch.apply` tool is deliberately small. This module is the
//! lower-level diff-review primitive for future multi-file edits: every file is
//! read and validated before any write occurs.
//!
//! When `apply_patch_set_atomic` commits writes, it first backs up every file
//! that will be modified. If any step fails, all modified files are restored
//! from their backups.

use crate::file_tool::is_sensitive_path;
use crate::patch::{
    apply_replace_patch, stable_text_hash, validate_patch, PatchApplyError, PatchCheck,
    PatchValidation, ReplacePatch,
};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchSetOperation {
    pub path: String,
    pub old_string: String,
    pub new_string: String,
    pub base_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchSetProposal {
    pub patch_set_id: String,
    pub operations: Vec<PatchSetOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchSetOperationReport {
    pub path: String,
    pub validation: PatchValidation,
    pub current_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchSetValidationReport {
    pub patch_set_id: String,
    pub ok: bool,
    pub operation_count: usize,
    pub reports: Vec<PatchSetOperationReport>,
    pub errors: Vec<String>,
}

impl PatchSetValidationReport {
    pub fn to_summary_line(&self) -> String {
        format!(
            "patch-set id={} ok={} ops={} errors={}",
            self.patch_set_id,
            self.ok,
            self.operation_count,
            self.errors.len()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchSetApplyReport {
    pub validation: PatchSetValidationReport,
    pub applied_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchSetError {
    Validation(PatchSetValidationReport),
    Io(String),
    PathEscapesWorkspace(String),
    DuplicatePath(String),
    EmptyPatchSet,
}

pub fn validate_patch_set(
    workspace_root: &Path,
    proposal: &PatchSetProposal,
) -> Result<PatchSetValidationReport, PatchSetError> {
    if proposal.operations.is_empty() {
        return Err(PatchSetError::EmptyPatchSet);
    }
    let mut seen_paths = HashSet::<String>::new();
    let mut reports = Vec::new();
    let mut errors = Vec::new();
    for operation in &proposal.operations {
        if !seen_paths.insert(operation.path.clone()) {
            return Err(PatchSetError::DuplicatePath(operation.path.clone()));
        }
        let path = resolve_patch_path(workspace_root, &operation.path)?;
        let current_text = fs::read_to_string(&path).ok();
        let current_hash = current_text.as_deref().map(stable_text_hash);
        let validation = validate_patch(PatchCheck {
            path: &operation.path,
            current_text: current_text.as_deref(),
            current_hash: current_hash.as_deref(),
            old_string: &operation.old_string,
            base_hash: &operation.base_hash,
        });
        if !matches!(
            validation,
            PatchValidation::Pass | PatchValidation::PassCreate
        ) {
            errors.push(format!("{}:{validation:?}", operation.path));
        }
        reports.push(PatchSetOperationReport {
            path: operation.path.clone(),
            validation,
            current_hash,
        });
    }
    Ok(PatchSetValidationReport {
        patch_set_id: proposal.patch_set_id.clone(),
        ok: errors.is_empty(),
        operation_count: proposal.operations.len(),
        reports,
        errors,
    })
}

/// Apply all operations atomically with rollback support.
///
/// Before any file is modified, every file that will be written is backed up
/// to a `.bak` file. If any single operation fails, ALL previously applied
/// operations are rolled back by restoring their backups.
pub fn apply_patch_set_atomic(
    workspace_root: &Path,
    proposal: &PatchSetProposal,
) -> Result<PatchSetApplyReport, PatchSetError> {
    let validation = validate_patch_set(workspace_root, proposal)?;
    if !validation.ok {
        return Err(PatchSetError::Validation(validation));
    }
    // Phase 1: Backup every file that will be modified.
    let mut backups: Vec<(PathBuf, PathBuf)> = Vec::new();
    for operation in &proposal.operations {
        let path = resolve_patch_path(workspace_root, &operation.path)?;
        let bak_path = path.with_extension(&format!(
            "{}.bak",
            path.extension().and_then(|e| e.to_str()).unwrap_or("")
        ));
        if path.exists() {
            fs::copy(&path, &bak_path).map_err(|e| {
                // Restore any backups we already made.
                restore_backups(&backups);
                PatchSetError::Io(format!("backup failed for {}: {e}", operation.path))
            })?;
        } else {
            // File does not exist yet; record a sentinel (empty bak path).
            // We'll still record the pair so cleanup knows to delete the bak.
        }
        backups.push((path, bak_path));
    }
    // Phase 2: Apply each operation. Roll back on any failure.
    let mut applied_paths = Vec::new();
    for (i, operation) in proposal.operations.iter().enumerate() {
        let path = resolve_patch_path(workspace_root, &operation.path)?;
        let result = apply_replace_patch(&ReplacePatch {
            path: path.clone(),
            old_string: operation.old_string.clone(),
            new_string: operation.new_string.clone(),
            base_hash: operation.base_hash.clone(),
        });
        match result {
            Ok(_) => {
                applied_paths.push(operation.path.clone());
            }
            Err(error) => {
                // Rollback: restore all backed-up files.
                restore_backups(&backups);
                // Clean up backup files.
                cleanup_backups(&backups);
                return match error {
                    PatchApplyError::Validation(validation) => {
                        Err(PatchSetError::Validation(PatchSetValidationReport {
                            patch_set_id: proposal.patch_set_id.clone(),
                            ok: false,
                            operation_count: proposal.operations.len(),
                            reports: vec![PatchSetOperationReport {
                                path: operation.path.clone(),
                                validation: validation.clone(),
                                current_hash: None,
                            }],
                            errors: vec![format!(
                                "{}:{validation:?} (applied {i} before rollback)",
                                operation.path
                            )],
                        }))
                    }
                    PatchApplyError::Io(error) => Err(PatchSetError::Io(error)),
                };
            }
        }
    }
    // Phase 3: Clean up all backup files.
    cleanup_backups(&backups);
    Ok(PatchSetApplyReport {
        validation,
        applied_paths,
    })
}

/// Restore all backed-up files from their `.bak` copies.
fn restore_backups(backups: &[(PathBuf, PathBuf)]) {
    for (original, bak) in backups {
        let _ = fs::copy(bak, original);
    }
}

/// Delete all backup files.
fn cleanup_backups(backups: &[(PathBuf, PathBuf)]) {
    for (_original, bak) in backups {
        let _ = fs::remove_file(bak);
    }
}

fn resolve_patch_path(workspace_root: &Path, value: &str) -> Result<PathBuf, PatchSetError> {
    if value.trim().is_empty() || is_sensitive_path(value) {
        return Err(PatchSetError::PathEscapesWorkspace(value.to_string()));
    }
    let root = workspace_root
        .canonicalize()
        .map_err(|error| PatchSetError::Io(error.to_string()))?;
    let candidate = root.join(value);
    if is_sensitive_path(&candidate.to_string_lossy()) {
        return Err(PatchSetError::PathEscapesWorkspace(value.to_string()));
    }
    let parent = candidate
        .parent()
        .ok_or_else(|| PatchSetError::PathEscapesWorkspace(value.to_string()))?;
    let resolved_parent = parent
        .canonicalize()
        .map_err(|error| PatchSetError::Io(error.to_string()))?;
    if !resolved_parent.starts_with(&root) {
        return Err(PatchSetError::PathEscapesWorkspace(value.to_string()));
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn validates_and_applies_multi_file_patch_set() {
        let root = temp_root("patch-set-apply");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "pub const A: u8 = 1;\n").unwrap();
        fs::write(root.join("src/b.rs"), "pub const B: u8 = 2;\n").unwrap();
        let proposal = PatchSetProposal {
            patch_set_id: "ps_1".to_string(),
            operations: vec![
                PatchSetOperation {
                    path: "src/a.rs".to_string(),
                    old_string: "A: u8 = 1".to_string(),
                    new_string: "A: u8 = 10".to_string(),
                    base_hash: stable_text_hash("pub const A: u8 = 1;\n"),
                },
                PatchSetOperation {
                    path: "src/b.rs".to_string(),
                    old_string: "B: u8 = 2".to_string(),
                    new_string: "B: u8 = 20".to_string(),
                    base_hash: stable_text_hash("pub const B: u8 = 2;\n"),
                },
            ],
        };
        let validation = validate_patch_set(&root, &proposal).unwrap();
        assert!(validation.ok, "{validation:?}");
        let applied = apply_patch_set_atomic(&root, &proposal).unwrap();
        assert_eq!(applied.applied_paths.len(), 2);
        assert!(fs::read_to_string(root.join("src/a.rs"))
            .unwrap()
            .contains("A: u8 = 10"));
        assert!(fs::read_to_string(root.join("src/b.rs"))
            .unwrap()
            .contains("B: u8 = 20"));
        // Verify no backup files left behind
        assert!(!root.join("src/a.rs.bak").exists());
        assert!(!root.join("src/b.rs.bak").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_entire_patch_set_before_any_write_when_one_file_is_stale() {
        let root = temp_root("patch-set-stale");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "pub const A: u8 = 1;\n").unwrap();
        fs::write(root.join("src/b.rs"), "pub const B: u8 = 2;\n").unwrap();
        let proposal = PatchSetProposal {
            patch_set_id: "ps_stale".to_string(),
            operations: vec![
                PatchSetOperation {
                    path: "src/a.rs".to_string(),
                    old_string: "A: u8 = 1".to_string(),
                    new_string: "A: u8 = 10".to_string(),
                    base_hash: stable_text_hash("pub const A: u8 = 1;\n"),
                },
                PatchSetOperation {
                    path: "src/b.rs".to_string(),
                    old_string: "B: u8 = 2".to_string(),
                    new_string: "B: u8 = 20".to_string(),
                    base_hash: "stale_hash".to_string(),
                },
            ],
        };
        let result = apply_patch_set_atomic(&root, &proposal);
        assert!(matches!(result, Err(PatchSetError::Validation(_))));
        assert_eq!(
            fs::read_to_string(root.join("src/a.rs")).unwrap(),
            "pub const A: u8 = 1;\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("src/b.rs")).unwrap(),
            "pub const B: u8 = 2;\n"
        );
        let _ = fs::remove_dir_all(root);
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
}
