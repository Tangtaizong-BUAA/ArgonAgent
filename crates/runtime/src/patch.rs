//! Read-before-write patch invariant validator.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchValidation {
    Pass,
    PassCreate,
    FailProtected,
    FailCreateExists,
    FailMissing,
    FailStale,
    FailMissingOldString,
    FailAmbiguous,
    FailMissingBaseHash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchCheck<'a> {
    pub path: &'a str,
    pub current_text: Option<&'a str>,
    pub current_hash: Option<&'a str>,
    pub old_string: &'a str,
    pub base_hash: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacePatch {
    pub path: PathBuf,
    pub old_string: String,
    pub new_string: String,
    pub base_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchApplyError {
    Validation(PatchValidation),
    Io(String),
}

pub fn validate_patch(check: PatchCheck<'_>) -> PatchValidation {
    validate_patch_inner(check, false)
}

pub fn validate_patch_allowing_protected(check: PatchCheck<'_>) -> PatchValidation {
    validate_patch_inner(check, true)
}

fn validate_patch_inner(check: PatchCheck<'_>, allow_protected: bool) -> PatchValidation {
    if !allow_protected && is_protected_path(check.path) {
        return PatchValidation::FailProtected;
    }
    if check.old_string.is_empty() {
        return if check.current_text.is_some() {
            PatchValidation::FailCreateExists
        } else {
            PatchValidation::PassCreate
        };
    }
    let Some(text) = check.current_text else {
        return PatchValidation::FailMissing;
    };
    // If the file exists and old_string is non-empty, base_hash must not be
    // empty to prevent bypassing staleness checks.
    if check.base_hash.is_empty() {
        return PatchValidation::FailMissingBaseHash;
    }
    if let Some(current_hash) = check.current_hash {
        // Reject magic-string base_hash that would bypass staleness checks.
        if check.base_hash == "__compute__" {
            return PatchValidation::FailStale;
        }
        if check.base_hash != current_hash {
            return PatchValidation::FailStale;
        }
    }
    let matches = text.matches(check.old_string).count();
    match matches {
        0 => PatchValidation::FailMissingOldString,
        1 => PatchValidation::Pass,
        _ => PatchValidation::FailAmbiguous,
    }
}

pub fn apply_replace_patch(patch: &ReplacePatch) -> Result<PatchValidation, PatchApplyError> {
    apply_replace_patch_inner(patch, false)
}

pub fn apply_replace_patch_allowing_protected(
    patch: &ReplacePatch,
) -> Result<PatchValidation, PatchApplyError> {
    apply_replace_patch_inner(patch, true)
}

fn apply_replace_patch_inner(
    patch: &ReplacePatch,
    allow_protected: bool,
) -> Result<PatchValidation, PatchApplyError> {
    let path_text = patch.path.to_string_lossy().to_string();
    let current_text = fs::read_to_string(&patch.path).ok();
    let current_hash = current_text.as_deref().map(stable_text_hash);
    let validation = validate_patch_inner(
        PatchCheck {
            path: &path_text,
            current_text: current_text.as_deref(),
            current_hash: current_hash.as_deref(),
            old_string: &patch.old_string,
            base_hash: &patch.base_hash,
        },
        allow_protected,
    );
    match validation {
        PatchValidation::Pass => {
            let text = current_text.unwrap_or_default();
            let next = text.replacen(&patch.old_string, &patch.new_string, 1);
            atomic_write(&patch.path, next.as_bytes()).map_err(io_error)?;
            Ok(validation)
        }
        PatchValidation::PassCreate => {
            if let Some(parent) = patch.path.parent() {
                fs::create_dir_all(parent).map_err(io_error)?;
            }
            atomic_write(&patch.path, patch.new_string.as_bytes()).map_err(io_error)?;
            Ok(validation)
        }
        _ => Err(PatchApplyError::Validation(validation)),
    }
}

/// Write to a temporary file next to the target, then atomically rename.
/// This preserves data integrity: either the old file remains intact or the
/// new file is fully written.
fn atomic_write(target: &Path, content: &[u8]) -> Result<(), io::Error> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no parent directory"))?;
    // Create a temp file in the same directory (guaranteeing same filesystem for
    // atomic rename).
    let mut tmp = tempfile::Builder::new()
        .prefix(&format!(
            ".{}.",
            target
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("patch")
        ))
        .suffix(".tmp")
        .tempfile_in(parent)?;
    io::Write::write_all(&mut tmp, content)?;
    io::Write::flush(&mut tmp)?;
    // Atomically replace the target.
    tmp.persist(target).map_err(|e| e.error)?;
    Ok(())
}

pub fn is_protected_path(path: &str) -> bool {
    path.starts_with("..")
        || path.contains("/.ssh/")
        || path.contains(".env")
        || path.contains("id_rsa")
        || path.contains("id_ed25519")
}

pub fn stable_text_hash(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv64_{hash:016x}")
}

fn io_error(error: io::Error) -> PatchApplyError {
    PatchApplyError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_passes() {
        assert_eq!(
            validate_patch(PatchCheck {
                path: "src/parser.ts",
                current_text: Some("retry_count = 3\n"),
                current_hash: Some("hash"),
                old_string: "retry_count = 3",
                base_hash: "hash",
            }),
            PatchValidation::Pass
        );
    }

    #[test]
    fn ambiguous_match_fails() {
        assert_eq!(
            validate_patch(PatchCheck {
                path: "src/parser.ts",
                current_text: Some("helper()\nhelper()\n"),
                current_hash: Some("hash"),
                old_string: "helper()",
                base_hash: "hash",
            }),
            PatchValidation::FailAmbiguous
        );
    }

    #[test]
    fn stale_hash_fails() {
        assert_eq!(
            validate_patch(PatchCheck {
                path: "src/parser.ts",
                current_text: Some("version = current\n"),
                current_hash: Some("new_hash"),
                old_string: "version = old",
                base_hash: "old_hash",
            }),
            PatchValidation::FailStale
        );
    }

    #[test]
    fn empty_base_hash_when_file_exists_fails() {
        assert_eq!(
            validate_patch(PatchCheck {
                path: "src/parser.ts",
                current_text: Some("retry_count = 3\n"),
                current_hash: Some("hash"),
                old_string: "retry_count = 3",
                base_hash: "",
            }),
            PatchValidation::FailMissingBaseHash
        );
    }

    #[test]
    fn protected_path_fails() {
        assert_eq!(
            validate_patch(PatchCheck {
                path: "../.ssh/config",
                current_text: None,
                current_hash: None,
                old_string: "",
                base_hash: "",
            }),
            PatchValidation::FailProtected
        );
    }

    #[test]
    fn protected_path_can_pass_after_permission_gate() {
        assert_eq!(
            validate_patch_allowing_protected(PatchCheck {
                path: ".env",
                current_text: Some("TOKEN=old\n"),
                current_hash: Some("hash"),
                old_string: "TOKEN=old",
                base_hash: "hash",
            }),
            PatchValidation::Pass
        );
    }

    #[test]
    fn applies_replace_patch_after_validation() {
        let path = std::env::temp_dir().join("researchcode-apply-replace-patch.txt");
        fs::write(&path, "retry_count = 3\n").unwrap();
        let base_hash = stable_text_hash("retry_count = 3\n");
        let validation = apply_replace_patch(&ReplacePatch {
            path: path.clone(),
            old_string: "retry_count = 3".to_string(),
            new_string: "retry_count = 5".to_string(),
            base_hash,
        })
        .unwrap();
        assert_eq!(validation, PatchValidation::Pass);
        assert_eq!(fs::read_to_string(&path).unwrap(), "retry_count = 5\n");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_stale_patch_before_write() {
        let path = std::env::temp_dir().join("researchcode-stale-replace-patch.txt");
        fs::write(&path, "version = current\n").unwrap();
        let result = apply_replace_patch(&ReplacePatch {
            path: path.clone(),
            old_string: "version = current".to_string(),
            new_string: "version = next".to_string(),
            base_hash: "old_hash".to_string(),
        });
        assert_eq!(
            result,
            Err(PatchApplyError::Validation(PatchValidation::FailStale))
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "version = current\n");
        let _ = fs::remove_file(path);
    }
}
