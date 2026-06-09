//! Safe read-only text search tool.

use crate::file_tool::is_sensitive_path;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// Maximum bytes to read from a single file during search.
const MAX_FILE_READ_BYTES: u64 = 1_048_576; // 1 MB

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    pub root: PathBuf,
    pub pattern: String,
    pub max_results: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    pub path: PathBuf,
    pub line_number: usize,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOutcome {
    pub matches: Vec<SearchMatch>,
    pub searched_files: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    EmptyPattern,
    PathNotFound(String),
    PathEscapesWorkspace(String),
    Io(String),
}

pub fn search_text(
    request: &SearchRequest,
    workspace_root: &Path,
) -> Result<Vec<SearchMatch>, SearchError> {
    Ok(search_text_with_outcome(request, workspace_root)?.matches)
}

pub fn search_text_with_outcome(
    request: &SearchRequest,
    workspace_root: &Path,
) -> Result<SearchOutcome, SearchError> {
    if request.pattern.trim().is_empty() {
        return Err(SearchError::EmptyPattern);
    }
    // Validate search root is within workspace_root.
    let ws_root = workspace_root
        .canonicalize()
        .map_err(|e| SearchError::Io(format!("cannot canonicalize workspace_root: {e}")))?;
    let requested_root = if request.root.is_absolute() {
        request.root.clone()
    } else {
        ws_root.join(&request.root)
    };
    let resolved_search_root = if requested_root.exists() {
        requested_root
            .canonicalize()
            .map_err(|e| SearchError::Io(format!("cannot resolve search target: {e}")))?
    } else {
        return Err(SearchError::PathNotFound(
            request.root.to_string_lossy().to_string(),
        ));
    };
    if !resolved_search_root.starts_with(&ws_root) {
        return Err(SearchError::PathEscapesWorkspace(
            request.root.to_string_lossy().to_string(),
        ));
    }
    let mut outcome = SearchOutcome {
        matches: Vec::new(),
        searched_files: 0,
        truncated: false,
    };
    if resolved_search_root.is_file() {
        search_file(&resolved_search_root, request, &mut outcome)?;
    } else {
        visit_dir(&resolved_search_root, &ws_root, request, &mut outcome)?;
    }
    Ok(outcome)
}

fn visit_dir(
    dir: &Path,
    workspace_root: &Path,
    request: &SearchRequest,
    outcome: &mut SearchOutcome,
) -> Result<(), SearchError> {
    if outcome.matches.len() >= request.max_results {
        outcome.truncated = true;
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if should_skip(&path, &name) {
            continue;
        }
        if path.is_dir() {
            // Skip symlinked directories that point outside workspace_root.
            if path.is_symlink() {
                if let Ok(resolved) = path.canonicalize() {
                    if !resolved.starts_with(workspace_root) {
                        continue;
                    }
                }
            }
            visit_dir(&path, workspace_root, request, outcome)?;
        } else {
            // Skip symlinked files that point outside workspace_root.
            if path.is_symlink() {
                if let Ok(resolved) = path.canonicalize() {
                    if !resolved.starts_with(workspace_root) {
                        continue;
                    }
                }
            }
            search_file(&path, request, outcome)?;
        }
        if outcome.matches.len() >= request.max_results {
            outcome.truncated = true;
            break;
        }
    }
    Ok(())
}

fn search_file(
    path: &Path,
    request: &SearchRequest,
    outcome: &mut SearchOutcome,
) -> Result<(), SearchError> {
    // Stream-read up to MAX_FILE_READ_BYTES instead of loading whole file.
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };
    if !metadata.is_file() {
        return Ok(());
    }
    outcome.searched_files += 1;
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(()),
    };
    let read_limit = MAX_FILE_READ_BYTES.min(metadata.len()).max(1);
    let mut buffer = Vec::with_capacity(read_limit as usize);
    file.take(MAX_FILE_READ_BYTES).read_to_end(&mut buffer).ok();
    let text = match String::from_utf8(buffer) {
        Ok(t) => t,
        Err(_) => return Ok(()),
    };
    for (index, line) in text.lines().enumerate() {
        if line.contains(&request.pattern) {
            outcome.matches.push(SearchMatch {
                path: path.to_path_buf(),
                line_number: index + 1,
                line: line.to_string(),
            });
            if outcome.matches.len() >= request.max_results {
                outcome.truncated = true;
                break;
            }
        }
    }
    Ok(())
}

fn should_skip(path: &Path, name: &str) -> bool {
    if is_sensitive_path(&path.to_string_lossy()) {
        return true;
    }
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".venv" | "__pycache__" | ".DS_Store"
    )
}

fn io_error(error: io::Error) -> SearchError {
    SearchError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn searches_text_files_and_skips_sensitive_files() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-search-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "hello\nneedle\n").unwrap();
        fs::write(root.join(".env"), "needle=secret\n").unwrap();
        let results = search_text_with_outcome(
            &SearchRequest {
                root: root.clone(),
                pattern: "needle".to_string(),
                max_results: 10,
            },
            &root,
        )
        .unwrap();
        assert_eq!(results.matches.len(), 1);
        assert_eq!(results.matches[0].line_number, 2);
        assert_eq!(results.searched_files, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_target_searches_only_that_file() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-search-file-{nonce}"));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("target.txt"), "needle here\n").unwrap();
        fs::write(root.join("src/other.txt"), "needle elsewhere\n").unwrap();
        let results = search_text_with_outcome(
            &SearchRequest {
                root: PathBuf::from("target.txt"),
                pattern: "needle".to_string(),
                max_results: 10,
            },
            &root,
        )
        .unwrap();
        assert_eq!(results.matches.len(), 1);
        assert_eq!(
            results.matches[0]
                .path
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "target.txt"
        );
        assert_eq!(results.searched_files, 1);
        assert!(!results.truncated);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_pattern_is_rejected() {
        let root = std::env::temp_dir();
        assert_eq!(
            search_text(
                &SearchRequest {
                    root: PathBuf::from("."),
                    pattern: "".to_string(),
                    max_results: 10,
                },
                &root,
            ),
            Err(SearchError::EmptyPattern)
        );
    }

    #[test]
    fn rejects_search_root_outside_workspace() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-search-ws-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let result = search_text(
            &SearchRequest {
                root: PathBuf::from("/etc"),
                pattern: "test".to_string(),
                max_results: 10,
            },
            &root,
        );
        assert!(matches!(result, Err(SearchError::PathEscapesWorkspace(_))));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_target_is_model_contract_error() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-search-missing-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let result = search_text(
            &SearchRequest {
                root: PathBuf::from("missing.txt"),
                pattern: "needle".to_string(),
                max_results: 10,
            },
            &root,
        );
        assert_eq!(
            result,
            Err(SearchError::PathNotFound("missing.txt".to_string()))
        );
        let _ = fs::remove_dir_all(&root);
    }
}
